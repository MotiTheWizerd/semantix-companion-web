// The Claude Code lane for Semantix Companion.
//
// A stdio JSON-lines server: the Rust core writes one request object per line
// on stdin, this process streams response events back as one JSON object per
// line on stdout. stderr is for logs only — stdout carries nothing but
// protocol lines.
//
// Isolation is the point: every session runs with `settingSources: []`, so
// the user's global ~/.claude hooks, settings and CLAUDE.md never load — the
// companion's own memory system is the only one in the room — while the
// user's local Claude Code login (keychain / ~/.claude/.credentials.json)
// still authenticates the calls. `tools: []` drops Claude Code's own built-in
// tools too: the companion's declared tools are the ONLY ones in reach.
//
// Tools bridge back down: each declared tool becomes an in-process MCP tool
// whose handler asks the Rust core to execute it (Rust owns the memory organ,
// the archive and the workspace guard) and awaits the result mid-turn.
//
// Wire shapes:
//   → { id, type: "query", conversationId, model, systemPrompt?, transcript,
//       userText, tools?, images? }
//   → { id, type: "toolResult", callId, content?, error? }
//   ← { id, event: "delta", text }
//   ← { id, event: "toolCall", callId, name, arguments }
//   ← { id, event: "usage", inputTokens, outputTokens }
//   ← { id, event: "done" }
//   ← { id, event: "error", message }

import { createInterface } from "node:readline";
import { randomUUID } from "node:crypto";
import { createSdkMcpServer, query, tool } from "@anthropic-ai/claude-agent-sdk";
import { shapeOf } from "./schema.mjs";

/** The MCP server name our tools hang off; it prefixes every tool id the
 *  model sees (`mcp__companion__recall_memory`). */
const SERVER_NAME = "companion";

/** A tool round-trip can outlast the SDK's default 60s stream-close timeout
 *  (a memory recall crosses the network). Raise it before any query runs. */
process.env.CLAUDE_CODE_STREAM_CLOSE_TIMEOUT ??= "180000";

/** conversationId → Claude session id, for `resume`. In-memory by design:
 *  after a restart the first turn re-seeds context from the transcript the
 *  Rust core sends anyway (the companion's SQLite archive is the truth). */
const sessions = new Map();

/** callId → { resolve } for tool calls awaiting a result from Rust. */
const toolWaiters = new Map();

function send(payload) {
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

function log(...parts) {
  process.stderr.write(`[sidecar] ${parts.join(" ")}\n`);
}

/** No live session to resume — fold the archived turns into the prompt so the
 *  conversation keeps its footing across app restarts. */
function compileFreshPrompt(transcript, userText) {
  if (!transcript.length) return userText;
  const history = transcript
    .map((turn) => `${turn.role === "user" ? "User" : "You"}: ${turn.text}`)
    .join("\n\n");
  return (
    "<conversation-so-far>\n" +
    "This conversation continues from earlier turns (your replies are marked \"You\"):\n\n" +
    `${history}\n` +
    "</conversation-so-far>\n\n" +
    userText
  );
}

/** Hand one tool call down to Rust and wait for its result. The promise is
 *  settled by the `toolResult` line, or rejected if the host goes away. */
function executeThroughHost(requestId, name, args) {
  const callId = randomUUID();
  send({
    id: requestId,
    event: "toolCall",
    callId,
    name,
    arguments: JSON.stringify(args ?? {}),
  });
  return new Promise((resolve, reject) => {
    toolWaiters.set(callId, { resolve, reject });
  });
}

/** Build the in-process MCP server for this turn's declared tools. */
function buildToolServer(requestId, declarations) {
  const tools = declarations.map((declaration) =>
    tool(
      declaration.name,
      declaration.description,
      shapeOf(declaration.parameters),
      async (args) => {
        try {
          const text = await executeThroughHost(requestId, declaration.name, args);
          return { content: [{ type: "text", text }] };
        } catch (error) {
          // A failed tool is a result the model reads and recovers from — it
          // never kills the turn. Same contract as the Semantix lane.
          return {
            content: [
              { type: "text", text: `Tool error: ${error?.message ?? String(error)}` },
            ],
            isError: true,
          };
        }
      },
    ),
  );
  return createSdkMcpServer({ name: SERVER_NAME, version: "0.1.0", tools });
}

/** A turn carrying images can't ride the plain string prompt — it needs the
 *  SDK's streaming-input form, one user message of content blocks. Images
 *  lead and the words follow, the order a vision model reads "look at this:…"
 *  in (same ordering the Semantix lane uses). */
function imagePrompt(text, images) {
  const content = images.map((image) => ({
    type: "image",
    source: { type: "base64", media_type: image.mediaType, data: image.data },
  }));
  if (text) content.push({ type: "text", text });
  return (async function* () {
    yield {
      type: "user",
      session_id: "",
      parent_tool_use_id: null,
      message: { role: "user", content },
    };
  })();
}

async function handleQuery(request) {
  const resume = sessions.get(request.conversationId);
  const images = request.images ?? [];
  const text = resume
    ? request.userText
    : compileFreshPrompt(request.transcript ?? [], request.userText);
  const prompt = images.length > 0 ? imagePrompt(text, images) : text;

  const declarations = request.tools ?? [];
  const options = {
    model: request.model,
    // No filesystem settings: no global hooks, no CLAUDE.md, no user config.
    settingSources: [],
    // No Claude Code built-ins either — the companion's own tools, or none.
    tools: [],
    includePartialMessages: true,
    // Tools need room to call, read the result and answer; without tools one
    // turn is the whole conversation.
    maxTurns: declarations.length > 0 ? 12 : 1,
  };
  if (declarations.length > 0) {
    options.mcpServers = { [SERVER_NAME]: buildToolServer(request.id, declarations) };
    // Pre-approve exactly our tools, then never prompt: anything else is
    // denied rather than escalated to a user who has no dialog to answer.
    options.allowedTools = declarations.map(
      (declaration) => `mcp__${SERVER_NAME}__${declaration.name}`,
    );
    options.permissionMode = "dontAsk";
  }
  if (resume) options.resume = resume;
  if (request.systemPrompt) options.systemPrompt = request.systemPrompt;

  let sawText = false;
  for await (const message of query({ prompt, options })) {
    if (message.type === "system" && message.subtype === "init") {
      sessions.set(request.conversationId, message.session_id);
      continue;
    }
    if (message.type === "stream_event") {
      const event = message.event;
      if (
        event?.type === "content_block_delta" &&
        event.delta?.type === "text_delta" &&
        event.delta.text
      ) {
        sawText = true;
        send({ id: request.id, event: "delta", text: event.delta.text });
      }
      continue;
    }
    if (message.type === "result") {
      // Keep resume anchored to the latest session id — a resumed session
      // gets a fresh id, and resuming a stale one forks the conversation.
      if (message.session_id) {
        sessions.set(request.conversationId, message.session_id);
      }
      if (message.subtype !== "success") {
        const detail =
          message.subtype === "error_max_turns"
            ? "the turn limit was reached"
            : (message.errorMessage ?? message.subtype ?? "unknown error");
        throw new Error(`Claude Code did not finish: ${detail}`);
      }
      // Partial streaming should have carried the text; if it didn't (SDK
      // behavior shift), fall back to the result's final text so the reply
      // is never silently empty.
      if (!sawText && typeof message.result === "string" && message.result) {
        send({ id: request.id, event: "delta", text: message.result });
      }
      if (message.usage) {
        send({
          id: request.id,
          event: "usage",
          inputTokens: message.usage.input_tokens ?? 0,
          outputTokens: message.usage.output_tokens ?? 0,
        });
      }
    }
  }
  send({ id: request.id, event: "done" });
}

/** Queries still streaming. stdin closing means the host is done SENDING —
 *  not that in-flight answers may be dropped; exit only once drained. */
let inFlight = 0;
let stdinClosed = false;

function exitWhenDrained() {
  if (stdinClosed && inFlight === 0) process.exit(0);
}

const stdin = createInterface({ input: process.stdin, crlfDelay: Infinity });

stdin.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;
  let request;
  try {
    request = JSON.parse(trimmed);
  } catch {
    log("unparseable request line dropped");
    return;
  }

  if (request.type === "toolResult") {
    const waiter = toolWaiters.get(request.callId);
    if (!waiter) {
      log(`tool result for unknown call ${request.callId}`);
      return;
    }
    toolWaiters.delete(request.callId);
    if (request.error) waiter.reject(new Error(request.error));
    else waiter.resolve(request.content ?? "");
    return;
  }

  if (request.type !== "query" || !request.id) {
    log(`unknown request type: ${request.type ?? "<none>"}`);
    return;
  }

  inFlight += 1;
  handleQuery(request)
    .catch((error) => {
      log(`query ${request.id} failed:`, error?.message ?? String(error));
      send({
        id: request.id,
        event: "error",
        message: error?.message ?? "Claude Code failed to answer.",
      });
    })
    .finally(() => {
      inFlight -= 1;
      exitWhenDrained();
    });
});

process.stdin.on("close", () => {
  stdinClosed = true;
  exitWhenDrained();
});

log(`ready (node ${process.version})`);
