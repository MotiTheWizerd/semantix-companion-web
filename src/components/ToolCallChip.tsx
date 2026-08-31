// The 📖 tool instrument chip — one line per tool call the model made while
// composing an assistant message (sibling of the 🧠 MemoryRecallChip). A
// failed call renders its error instead of vanishing — same silence-is-a-bug
// contract as the memory chip. Pure render-view over ToolCallChipItem[].

import type { ToolCallChipItem } from "../features/chat/types";

const MEMORY_VERBS: Record<string, [verb: string, argument: string, fallback: string]> = {
  recall_memory: ["recalled", "name", "recalled a memory"],
  carve_memory: ["carved", "name", "carved a memory"],
  search_conversations: ["searched past conversations for", "query", "searched past conversations"],
  web_search: ["searched the web for", "query", "searched the web"],
  web_fetch: ["read", "url", "read a web page"],
};

const ICONS: Record<string, string> = {
  carve_memory: "🪶",
  search_conversations: "🔍",
  web_search: "🌐",
  web_fetch: "📄",
};

function callLabel(call: ToolCallChipItem): string {
  const labels = MEMORY_VERBS[call.name];
  if (!labels) return call.name;
  const [verb, argument, fallback] = labels;
  try {
    const parsed = JSON.parse(call.arguments) as Record<string, unknown>;
    const value = parsed[argument];
    return typeof value === "string" && value ? `${verb} [${value}]` : fallback;
  } catch {
    return fallback;
  }
}

function callIcon(call: ToolCallChipItem): string {
  return ICONS[call.name] ?? "📖";
}

export function ToolCallChip({ calls }: { calls: ToolCallChipItem[] }) {
  if (calls.length === 0) return null;
  return (
    <div className="tool-chip" role="status">
      {calls.map((call) => (
        <span
          key={call.callId}
          className={`tool-chip__call tool-chip__call--${call.status}`}
          title={call.arguments}
        >
          <span aria-hidden="true">{callIcon(call)}</span>
          <span>{callLabel(call)}</span>
          {call.status === "running" && (
            <span className="tool-chip__working" aria-hidden="true">
              <span />
              <span />
              <span />
            </span>
          )}
          {call.status === "error" && (
            <span className="tool-chip__error">⚠ {call.detail ?? "failed"}</span>
          )}
        </span>
      ))}
    </div>
  );
}
