// The sky's colours for the engine: the tints from tints.ts, promoted to
// three.js Colors and cached. Anything that does not draw should import
// tints.ts directly — this file costs three.

import { Color } from "three";

import { DEFAULT_TINT, TYPE_TINTS } from "./tints";

export { typeTintCss, TYPE_ORDER } from "./tints";

export const VOID_COLOR = "#06051a";

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
