// Composition of recalled hits into the <agent-memory> block that rides ahead
// of the user's message — ported from the studio module (s479-s482) so both
// products inject one proven shape. The etiquette carves out the meta-question
// on purpose (s482): a blanket "never mention this block" made models deny
// having memory at all.

import type { MemoryHit } from "./organService";
import { formatAge } from "./time";

/** Cap a memory body so one fat memory can't eat the whole prompt. */
export const MEMORY_BODY_CAP = 700;

function memoryRow({ memory }: MemoryHit, nowMs: number, marker = ""): string {
  const body =
    memory.body.length > MEMORY_BODY_CAP
      ? `${memory.body.slice(0, MEMORY_BODY_CAP)}…`
      : memory.body;
  const age = formatAge(memory.created_at, nowMs);
  const carved = age ? ` · carved ${age} (${memory.created_at.slice(0, 10)})` : "";
  return `${marker}[${memory.name}] (${memory.mem_type}${carved}) — ${memory.description}\n${body}`;
}

export function composeMemoryBlock(
  hits: MemoryHit[],
  mirrorHits: MemoryHit[],
  nowMs: number,
): string {
  if (hits.length === 0 && mirrorHits.length === 0) return "";
  const parts: string[] = [];
  if (hits.length > 0)
    parts.push(hits.map((hit) => memoryRow(hit, nowMs)).join("\n\n"));
  if (mirrorHits.length > 0)
    parts.push(
      `The following surfaced in response to YOUR OWN previous reply, not the ` +
        `user's message. They are associations, not confirmations — if one ` +
        `contradicts what you said, that is the signal worth having.\n\n` +
        mirrorHits.map((hit) => memoryRow(hit, nowMs, "🪞 ")).join("\n\n"),
    );
  return (
    `<agent-memory>\n` +
    `Your own long-term memory, recalled for this message — not a tool result, ` +
    `and unrelated to any separate memory tool you may have. Draw on it ` +
    `naturally when relevant: never recite it back, quote this block, or ` +
    `mention its existence unprompted. If the user asks directly whether you ` +
    `remember something or have memory at all, answer honestly from what is ` +
    `here rather than denying it.\n\n` +
    `${parts.join("\n\n")}\n` +
    `</agent-memory>\n\n`
  );
}
