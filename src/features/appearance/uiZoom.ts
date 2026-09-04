// Ctrl +/- — the whole interface, larger or smaller.
//
// ⚑ WHY ZOOM AND NOT A FONT-SIZE ANCHOR. The usual trick is to move
// `html { font-size }` and let every `rem` follow it. That does not work here:
// this app states its sizes in PIXELS (98 px font-sizes against 17 rem when
// this was written) and has no root font-size anchor at all, so moving the
// anchor would resize a handful of surfaces and leave the rest exactly where
// they were — a control that looks broken rather than one that is off.
//
// Zoom scales type AND the space around it, which is also what a person means
// by "make it bigger": text alone growing inside fixed-size chrome gets
// cramped, then clipped. Converting the stylesheet to rem is the deeper fix
// and a real piece of work; when it happens, this file is what it replaces,
// and the keymap below stays as it is.

const STORAGE_KEY = "semantix.companion.uiZoom";

/** 100% is the design size. The range is what stays usable: below 70% the
 *  chrome's fixed paddings crowd the text, above 200% the composer stops
 *  fitting a sentence. */
export const MIN_ZOOM = 0.7;
export const MAX_ZOOM = 2;
export const DEFAULT_ZOOM = 1;

/** Multiplicative, not additive: a 10% step feels the same size at 70% as it
 *  does at 200%, where a flat +0.1 would be a lurch at the bottom and a
 *  rounding error at the top. */
const STEP = 1.1;

function clamp(zoom: number): number {
  if (!Number.isFinite(zoom)) return DEFAULT_ZOOM;
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom));
}

/** Snap to whole percents so repeated stepping cannot drift into 1.0999999. */
function round(zoom: number): number {
  return Math.round(zoom * 100) / 100;
}

/** A view preference, so it lives in the browser store rather than the
 *  companion database: it is per-screen rather than per-companion, wanted the
 *  instant a key is pressed, and nothing on the Rust side has any use for it.
 *  A private window or cleared storage simply starts at 100%. */
export function readZoom(): number {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored === null ? DEFAULT_ZOOM : clamp(Number.parseFloat(stored));
  } catch {
    return DEFAULT_ZOOM;
  }
}

function writeZoom(zoom: number): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, String(zoom));
  } catch {
    // Storage can be unavailable or full; the zoom still applies for this run.
  }
}

/** Put the value on the document. `zoom` is deliberately chosen over
 *  `transform: scale()`: it reflows the layout at the new size, so the app
 *  still fills the window and nothing overflows, where a transform would
 *  scale a picture of the old layout and leave scrollbars behind. */
export function applyZoom(zoom: number): void {
  document.documentElement.style.zoom = zoom === 1 ? "" : String(zoom);
}

export function setZoom(zoom: number): number {
  const next = round(clamp(zoom));
  applyZoom(next);
  writeZoom(next);
  return next;
}

export function zoomIn(): number {
  return setZoom(readZoom() * STEP);
}

export function zoomOut(): number {
  return setZoom(readZoom() / STEP);
}

export function resetZoom(): number {
  return setZoom(DEFAULT_ZOOM);
}

/**
 * Which zoom action a keystroke means, or `null` for every other key.
 *
 * ⚑ THE KEY IS NOT THE CHARACTER. `Ctrl +` is typed as `Ctrl Shift =` on a US
 * layout and as its own key on a numpad, and browsers report all of these
 * differently — so both `key` and `code` are consulted. Getting this wrong
 * gives a shortcut that works on the author's keyboard and nobody else's.
 */
export function zoomIntent(event: KeyboardEvent): "in" | "out" | "reset" | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey) return null;

  switch (event.key) {
    case "+":
    case "=":
      return "in";
    case "-":
    case "_":
      return "out";
    case "0":
      return "reset";
    default:
      break;
  }

  // Numpad keys report `key` as the digit or operator above, but a layout that
  // does not produce them still reports a stable `code`.
  switch (event.code) {
    case "NumpadAdd":
    case "Equal":
      return "in";
    case "NumpadSubtract":
    case "Minus":
      return "out";
    case "Numpad0":
    case "Digit0":
      return "reset";
    default:
      return null;
  }
}
