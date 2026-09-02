// The sky's colours — one family, the lightning school: white-blue cores,
// electric blue, violet fringe, a deep indigo void. A memory's TYPE is a tint
// inside that family, not a category colour from a chart; the sky has to read
// as one spell, not a legend.

import { Color } from "three";

export const VOID_COLOR = "#06051a";

/** Tint per memory type. Anything unknown (Muninn carries types the organ
 *  does not, e.g. `visual`) falls to the lavender default. */
const TYPE_TINTS: Record<string, string> = {
  insight: "#a97dff", // violet — the spell's fringe
  project: "#56b8ff", // electric blue — the work itself
  reference: "#5ff0e4", // cold cyan — pointers, cool and exact
  episodic: "#cfe4ff", // ice white — what happened
  feedback: "#ff7ad9", // magenta — a correction leaves a mark
  user: "#ffd58a", // pale gold — people are warm
};
const DEFAULT_TINT = "#b8a8ff";

const cache = new Map<string, Color>();

export function typeColor(memType: string): Color {
  const key = memType in TYPE_TINTS ? memType : DEFAULT_TINT;
  let color = cache.get(key);
  if (!color) {
    color = new Color(TYPE_TINTS[key] ?? DEFAULT_TINT);
    cache.set(key, color);
  }
  return color;
}

export function typeTintCss(memType: string): string {
  return TYPE_TINTS[memType] ?? DEFAULT_TINT;
}

export const TYPE_ORDER = ["insight", "project", "reference", "episodic", "feedback", "user"];
