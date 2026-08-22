// Time awareness — the now-anchor + the gap reflex ride ahead of the memory
// block, so the model never has to subtract timestamps to notice the user
// left and came back.

import { composeTimeBlock } from "../time";
import type { MemoryReflex } from "./types";

export const timeAwareness: MemoryReflex = {
  id: "time-awareness",
  label: "Time awareness",
  description: "Anchor the model in now — and call out long gaps between messages",
  defaultOn: true,
  run(ctx) {
    return Promise.resolve({
      kind: "block" as const,
      text: composeTimeBlock(ctx.nowMs, ctx.lastUserAtMs),
    });
  },
};
