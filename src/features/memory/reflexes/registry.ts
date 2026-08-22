// The reflex registry + the one pre-send pass the chat calls — ported from the
// studio module. Adding a sense = one reflex file + one entry here + its pref
// default; nothing else in the send path ever changes.

import { composeMemoryBlock } from "../compose";
import type { MemoryHit } from "../organService";
import { isMemoryEnabled, reflexSetting, type MemoryPrefs } from "../prefs";
import { promptRecall } from "./promptRecall";
import { selfRecall } from "./selfRecall";
import { timeAwareness } from "./timeAwareness";
import type { MemoryReflex, ReflexCtx, ReflexRunReport } from "./types";

/** Declaration order is composition order: 'block' contributions ride first
 *  (time before memories), then the single <agent-memory> block. */
export const memoryReflexes: MemoryReflex[] = [
  timeAwareness,
  promptRecall,
  selfRecall,
];

const EMPTY_REPORT: ReflexRunReport = {
  injection: "",
  hits: [],
  mirrorHits: [],
  vectorLeg: null,
  ran: [],
  errors: [],
};

/** Run every enabled reflex in parallel, fail-open per reflex, and compose one
 *  injection string. A dead organ, a disabled master switch, or an empty
 *  registry all degrade to "inject nothing". */
export async function runPreSendReflexes(
  ctx: ReflexCtx,
  prefs: MemoryPrefs,
): Promise<ReflexRunReport> {
  if (!isMemoryEnabled(prefs)) return EMPTY_REPORT;
  const enabled = memoryReflexes.filter((reflex) => reflexSetting(prefs, reflex).enabled);
  if (enabled.length === 0) return EMPTY_REPORT;

  const settled = await Promise.allSettled(enabled.map((reflex) => reflex.run(ctx)));

  const blocks: string[] = [];
  const hits: MemoryHit[] = [];
  const riding = new Set<string>();
  const mirrorRaw: { hits: MemoryHit[]; cap?: number }[] = [];
  let vectorLeg: boolean | null = null;
  const errors: ReflexRunReport["errors"] = [];

  settled.forEach((outcome, i) => {
    if (outcome.status === "rejected") {
      const reason: unknown = outcome.reason;
      errors.push({
        reflexId: enabled[i].id,
        message: reason instanceof Error ? reason.message : String(reason),
      });
      return;
    }
    const contribution = outcome.value;
    if (!contribution) return;
    if (contribution.kind === "block") {
      if (contribution.text) blocks.push(contribution.text);
      return;
    }
    if (contribution.vectorLeg !== undefined)
      vectorLeg = vectorLeg === false ? false : contribution.vectorLeg;
    if (contribution.mirror) {
      mirrorRaw.push({ hits: contribution.hits, cap: contribution.cap });
      return;
    }
    for (const hit of contribution.hits) {
      if (riding.has(hit.memory.id)) continue;
      riding.add(hit.memory.id);
      hits.push(hit);
    }
  });

  // Mirror hits land after every prompt-leg hit is known: dedupe, then cap.
  const mirrorHits: MemoryHit[] = [];
  for (const { hits: candidates, cap } of mirrorRaw) {
    const fresh = candidates.filter((hit) => !riding.has(hit.memory.id));
    for (const hit of cap === undefined ? fresh : fresh.slice(0, cap)) {
      riding.add(hit.memory.id);
      mirrorHits.push(hit);
    }
  }

  return {
    injection: blocks.join("") + composeMemoryBlock(hits, mirrorHits, ctx.nowMs),
    hits,
    mirrorHits,
    vectorLeg,
    ran: enabled.map((reflex) => reflex.id),
    errors,
  };
}
