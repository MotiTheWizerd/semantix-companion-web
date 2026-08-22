// The 🧠 instrument chip — a collapsed one-liner under a sent message showing
// what the memory reflexes handed the model that turn (Companion sibling of
// the studio's MemoryRecallChip). Click to expand the hit list; a failed pass
// renders as a warning instead of vanishing — silence is what hid the s483
// organ break. Pure render-view over MemoryRecallChipData.

import { useState } from "react";

import type { MemoryRecallChipData } from "../features/memory";

export function MemoryRecallChip({ data }: { data: MemoryRecallChipData }) {
  const [open, setOpen] = useState(false);

  if (data.failure) {
    return (
      <div className="memory-chip memory-chip--failed" role="status">
        <span aria-hidden="true">🧠</span>
        <span>⚠ memory unavailable — {data.failure}</span>
      </div>
    );
  }

  const mirrorCount = data.hits.filter((hit) => hit.mirror).length;
  const promptCount = data.hits.length - mirrorCount;
  const summary = [
    `${promptCount} memor${promptCount === 1 ? "y" : "ies"}`,
    ...(mirrorCount > 0 ? [`🪞 ${mirrorCount}`] : []),
    ...(data.vectorLeg === false ? ["⚠ keyword-only"] : []),
    ...(data.errors.length > 0 ? [`⚠ ${data.errors.length} reflex failed`] : []),
  ].join(" · ");

  return (
    <div className="memory-chip" role="status">
      <button
        type="button"
        className="memory-chip__toggle"
        onClick={() => setOpen((value) => !value)}
        title="Memories recalled for this message — what the model was handed"
      >
        <span aria-hidden="true">🧠</span>
        <span>{summary}</span>
        <span aria-hidden="true" className="memory-chip__chevron">
          {open ? "▾" : "▸"}
        </span>
      </button>
      {open && (
        <ul className="memory-chip__hits">
          {data.hits.map((hit) => (
            <li key={`${hit.name}${hit.mirror ? ":m" : ""}`}>
              {hit.mirror && <span title="cued by the agent's own last reply">🪞 </span>}
              <span className="memory-chip__name">[{hit.name}]</span>{" "}
              <span className="memory-chip__meta">
                {hit.memType} · {hit.score.toFixed(2)}
              </span>
            </li>
          ))}
          {data.errors.map((error) => (
            <li key={`err:${error.reflexId}`} className="memory-chip__error">
              ⚠ {error.reflexId}: {error.message}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
