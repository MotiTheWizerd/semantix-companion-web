// The sky's tints as plain CSS strings — one family, the lightning school:
// white-blue cores, electric blue, violet fringe, a deep indigo void. A
// memory's TYPE is a tint inside that family, not a category colour from a
// chart; the sky has to read as one spell, not a legend.
//
// This file must never import three: the sidebar's legend reads it, and the
// sidebar is on the chat's bundle. The engine's `Color` objects live next
// door in palette.ts.

/** Tint per memory type. Anything unknown (Muninn carries types the organ
 *  does not, e.g. `visual`) falls to the lavender default. */
export const TYPE_TINTS: Record<string, string> = {
  insight: "#a97dff", // violet — the spell's fringe
  project: "#56b8ff", // electric blue — the work itself
  reference: "#5ff0e4", // cold cyan — pointers, cool and exact
  episodic: "#cfe4ff", // ice white — what happened
  feedback: "#ff7ad9", // magenta — a correction leaves a mark
  user: "#ffd58a", // pale gold — people are warm
};

export const DEFAULT_TINT = "#b8a8ff";

export function typeTintCss(memType: string): string {
  return TYPE_TINTS[memType] ?? DEFAULT_TINT;
}

export const TYPE_ORDER = ["insight", "project", "reference", "episodic", "feedback", "user"];
