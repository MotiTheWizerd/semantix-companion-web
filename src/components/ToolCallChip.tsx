// The 📖 tool instrument chip — one line per tool call the model made while
// composing an assistant message (sibling of the 🧠 MemoryRecallChip). A
// failed call renders its error instead of vanishing — same silence-is-a-bug
// contract as the memory chip. Pure render-view over ToolCallChipItem[].

import type { ToolCallChipItem } from "../features/chat/types";

function callLabel(call: ToolCallChipItem): string {
  if (call.name !== "recall_memory") return call.name;
  try {
    const parsed = JSON.parse(call.arguments) as { name?: unknown };
    return typeof parsed.name === "string" && parsed.name
      ? `recalled [${parsed.name}]`
      : "recalled a memory";
  } catch {
    return "recalled a memory";
  }
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
          <span aria-hidden="true">📖</span>
          <span>{callLabel(call)}</span>
          {call.status === "running" && <span aria-hidden="true">…</span>}
          {call.status === "error" && (
            <span className="tool-chip__error">⚠ {call.detail ?? "failed"}</span>
          )}
        </span>
      ))}
    </div>
  );
}
