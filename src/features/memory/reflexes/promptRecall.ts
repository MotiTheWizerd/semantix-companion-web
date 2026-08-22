// Prompt recall — the primary sense: the user's message cues the organ's
// hybrid recall and the hits ride into the model's prompt.

import { recallMemories } from "../organService";
import type { MemoryReflex } from "./types";

/** Server caps recall queries at 8000 chars. */
const QUERY_CAP = 8000;

export const promptRecall: MemoryReflex = {
  id: "prompt-recall",
  label: "Prompt recall",
  description: "Recall memories cued by the user's message",
  defaultOn: true,
  async run(ctx) {
    const query = ctx.text.trim();
    if (!query) return null;
    const result = await recallMemories(ctx.agentId, query.slice(0, QUERY_CAP));
    return { kind: "hits", hits: result.hits, vectorLeg: result.vector_leg };
  },
};
