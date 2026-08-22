// Self-recall — the mirror leg: a second recall cued by the AGENT'S OWN last
// reply, gated to short prompts ("ok do it" tells the organ nothing, but the
// reply it follows is dense with cues). Its real function is a contradiction
// detector. Dedupe against the prompt leg + the cap happen in the composer.

import { recallMemories } from "../organService";
import type { MemoryReflex } from "./types";

const PROMPT_GATE = 200;
const QUERY_CAP = 2000;
const MAX_HITS = 2;

export const selfRecall: MemoryReflex = {
  id: "self-recall",
  label: "Self-recall (mirror)",
  description: "Check the agent's own last reply against memory on short prompts",
  defaultOn: true,
  async run(ctx) {
    if (!ctx.lastAssistantText || ctx.text.length >= PROMPT_GATE) return null;
    // Over-fetch so dedupe against the prompt leg still leaves hits to land.
    const result = await recallMemories(
      ctx.agentId,
      ctx.lastAssistantText.slice(0, QUERY_CAP),
      MAX_HITS + 4,
    );
    return { kind: "hits", hits: result.hits, mirror: true, cap: MAX_HITS };
  },
};
