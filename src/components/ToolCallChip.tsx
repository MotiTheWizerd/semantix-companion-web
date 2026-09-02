// The 📖 tool instrument chip — one glass pill per tool call the model made
// while composing an assistant message (sibling of the 🧠 MemoryRecallChip).
// A pill wears its tool's FAMILY tint, the memory sky's own colours: web is
// cyan like a reference, files violet like an insight, mail and calls gold
// like a person — a tool looks like what it will become. State lives in the
// orb at the pill's end, never in opacity: blue and breathing while the call
// runs, cyan once it lands, rose when it fails (the error text rides along —
// same silence-is-a-bug contract as the memory chip). Pure render-view over
// ToolCallChipItem[]; memory tools never reach it (the backend emits no chip
// for them, s540).

import type { ReactElement } from "react";

import type { ToolCallChipItem } from "../features/chat/types";

type Family = "web" | "file" | "people" | "tool";
type Icon = "globe" | "page" | "folder" | "file" | "mail" | "phone" | "users" | "wrench";
type Argument =
  | { kind: "text"; key: string }
  | { kind: "url"; key: string }
  | { kind: "path"; key: string }
  | { kind: "agent"; key: string }
  | { kind: "none" };

interface ToolFace {
  family: Family;
  icon: Icon;
  /** Present tense while the call runs, past once it has landed. */
  running: string;
  done: string;
  argument: Argument;
}

const FACES: Record<string, ToolFace> = {
  web_search: {
    family: "web",
    icon: "globe",
    running: "searching",
    done: "searched",
    argument: { kind: "text", key: "query" },
  },
  web_fetch: {
    family: "web",
    icon: "page",
    running: "reading",
    done: "read",
    argument: { kind: "url", key: "url" },
  },
  list_files: {
    family: "file",
    icon: "folder",
    running: "listing",
    done: "listed",
    argument: { kind: "path", key: "path" },
  },
  read_file: {
    family: "file",
    icon: "file",
    running: "reading",
    done: "read",
    argument: { kind: "path", key: "path" },
  },
  write_file: {
    family: "file",
    icon: "file",
    running: "writing",
    done: "wrote",
    argument: { kind: "path", key: "path" },
  },
  edit_file: {
    family: "file",
    icon: "file",
    running: "editing",
    done: "edited",
    argument: { kind: "path", key: "path" },
  },
  delete_file: {
    family: "file",
    icon: "file",
    running: "deleting",
    done: "deleted",
    argument: { kind: "path", key: "path" },
  },
  list_agents: {
    family: "people",
    icon: "users",
    running: "looking up companions",
    done: "looked up companions",
    argument: { kind: "none" },
  },
  send_message: {
    family: "people",
    icon: "mail",
    running: "writing to",
    done: "wrote to",
    argument: { kind: "agent", key: "to_agent_id" },
  },
  read_messages: {
    family: "people",
    icon: "mail",
    running: "reading the inbox",
    done: "read the inbox",
    argument: { kind: "none" },
  },
  mark_message_read: {
    family: "people",
    icon: "mail",
    running: "marking a message read",
    done: "marked a message read",
    argument: { kind: "none" },
  },
  open_call: {
    family: "people",
    icon: "phone",
    running: "calling",
    done: "called",
    argument: { kind: "agent", key: "to_agent_id" },
  },
  send_in_call: {
    family: "people",
    icon: "phone",
    running: "speaking in the call",
    done: "spoke in the call",
    argument: { kind: "none" },
  },
  read_call: {
    family: "people",
    icon: "phone",
    running: "reading the call",
    done: "read the call",
    argument: { kind: "none" },
  },
  list_calls: {
    family: "people",
    icon: "phone",
    running: "listing calls",
    done: "listed calls",
    argument: { kind: "none" },
  },
  // The memory tools are invisible by design; these only matter if one ever
  // slips through, so the chip still says something true.
  recall_memory: {
    family: "tool",
    icon: "wrench",
    running: "recalling",
    done: "recalled",
    argument: { kind: "text", key: "name" },
  },
  carve_memory: {
    family: "tool",
    icon: "wrench",
    running: "carving",
    done: "carved",
    argument: { kind: "text", key: "name" },
  },
  search_conversations: {
    family: "tool",
    icon: "wrench",
    running: "searching past conversations for",
    done: "searched past conversations for",
    argument: { kind: "text", key: "query" },
  },
};

/** A tool the chip has no face for still gets an honest pill: its raw name
 *  as the verb, the generic tint, no argument. */
function faceFor(name: string): ToolFace {
  return (
    FACES[name] ?? {
      family: "tool",
      icon: "wrench",
      running: name,
      done: name,
      argument: { kind: "none" },
    }
  );
}

function parsedArgument(call: ToolCallChipItem, key: string): string | null {
  try {
    const parsed = JSON.parse(call.arguments) as Record<string, unknown>;
    const value = parsed[key];
    return typeof value === "string" && value.trim() ? value.trim() : null;
  } catch {
    return null;
  }
}

/** A url reads as its host and path — no scheme, no query, no "www.", no
 *  trailing slash — so two fetches of one site still tell apart. */
function shortUrl(raw: string): string {
  try {
    const url = new URL(raw);
    const host = url.hostname.replace(/^www\./, "");
    const path = url.pathname.replace(/\/+$/, "");
    return path ? `${host}${path}` : host;
  } catch {
    return raw;
  }
}

/** A path reads as its last two segments — the filename and the folder it
 *  sits in — so an end-ellipsis never eats the name. */
function shortPath(raw: string): string {
  const parts = raw.replace(/\/+$/, "").split("/").filter(Boolean);
  return parts.length > 2 ? parts.slice(-2).join("/") : parts.join("/") || raw;
}

function argumentLabel(
  call: ToolCallChipItem,
  face: ToolFace,
  agentNames: ReadonlyMap<string, string> | undefined,
): string | null {
  const argument = face.argument;
  if (argument.kind === "none") return null;
  const value = parsedArgument(call, argument.key);
  if (!value) return argument.kind === "agent" ? "a companion" : null;
  switch (argument.kind) {
    case "url":
      return shortUrl(value);
    case "path":
      return shortPath(value);
    case "agent":
      return agentNames?.get(value) ?? "a companion";
    case "text":
      return value;
  }
}

/** Elapsed time the way a stopwatch shows it: tenths under ten seconds,
 *  whole seconds after, minutes once it is that kind of call. */
export function formatElapsed(ms: number): string {
  const seconds = ms / 1000;
  if (seconds < 10) return `${seconds.toFixed(1)}s`;
  if (seconds < 60) return `${Math.round(seconds)}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds - minutes * 60);
  return `${minutes}m ${rest.toString().padStart(2, "0")}s`;
}

const ICON_PATHS: Record<Icon, ReactElement> = {
  globe: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
    </>
  ),
  page: (
    <>
      <path d="M14 3H7a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V8z" />
      <path d="M14 3v5h5M9 13h6M9 17h6" />
    </>
  ),
  folder: (
    <path d="M4 6a1 1 0 0 1 1-1h5l2 2h7a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1z" />
  ),
  file: (
    <>
      <path d="M14 3H7a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h10a1 1 0 0 0 1-1V8z" />
      <path d="M14 3v5h5" />
    </>
  ),
  mail: (
    <>
      <rect x="3" y="5" width="18" height="14" rx="2" />
      <path d="m3 7 9 6 9-6" />
    </>
  ),
  phone: (
    <path d="M5 4h4l2 5-2.5 1.5a11 11 0 0 0 5 5L15 13l5 2v4a2 2 0 0 1-2 2A16 16 0 0 1 3 6a2 2 0 0 1 2-2" />
  ),
  users: (
    <>
      <circle cx="9" cy="8" r="3.5" />
      <path d="M2.5 20a6.5 6.5 0 0 1 13 0M16 5a3.5 3.5 0 0 1 0 7M21.5 20a6.5 6.5 0 0 0-4.5-6.2" />
    </>
  ),
  wrench: (
    <path d="M14.5 6.5a4 4 0 0 0 5 5L13 18a2.5 2.5 0 0 1-3.5-3.5l6.5-6.5a4 4 0 0 0-1.5-1.5z" />
  ),
};

function ToolIcon({ icon }: { icon: Icon }) {
  return (
    <svg className="tool-chip__icon" viewBox="0 0 24 24" aria-hidden="true">
      {ICON_PATHS[icon]}
    </svg>
  );
}

export function ToolCallChip({
  calls,
  agentNames,
}: {
  calls: ToolCallChipItem[];
  /** Agent id → display name, so "wrote to Hugin" instead of a uuid. */
  agentNames?: ReadonlyMap<string, string>;
}) {
  if (calls.length === 0) return null;
  return (
    <div className="tool-chip" role="status">
      {calls.map((call) => {
        const face = faceFor(call.name);
        const running = call.status === "running";
        const argument = argumentLabel(call, face, agentNames);
        return (
          <span
            key={call.callId}
            className={`tool-chip__call tool-chip__call--${call.status} tool-chip__call--${face.family}`}
            title={call.arguments}
          >
            <ToolIcon icon={face.icon} />
            <span className="tool-chip__verb">{running ? face.running : face.done}</span>
            {argument ? <span className="tool-chip__arg">{argument}</span> : null}
            {call.status === "error" ? (
              <span className="tool-chip__why" title={call.detail ?? undefined}>
                {call.detail ?? "failed"}
              </span>
            ) : null}
            {call.status === "ok" && call.elapsedMs !== null ? (
              <span className="tool-chip__time">{formatElapsed(call.elapsedMs)}</span>
            ) : null}
            <span className="tool-chip__orb" aria-hidden="true" />
          </span>
        );
      })}
    </div>
  );
}
