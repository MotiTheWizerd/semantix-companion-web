import { useEffect } from "react";

import { applyZoom, readZoom, resetZoom, zoomIn, zoomIntent, zoomOut } from "./uiZoom";

/**
 * Ctrl/Cmd +, -, and 0 for the whole interface, restored on every launch.
 *
 * Mounted once at the app root. The listener sits on `window` in the CAPTURE
 * phase so it answers wherever focus is — the composer, a dialog, the sky —
 * without every one of those surfaces having to know the shortcut exists.
 *
 * ⚑ `preventDefault` matters here. Ctrl +/- is the webview's OWN zoom, and
 * leaving it unhandled means both zooms apply at once: the app scales, the
 * webview scales, and the two drift apart with no way to get back in step.
 */
export function useUiZoom(): void {
  useEffect(() => {
    // The stored value is applied before the first paint of the effect rather
    // than left to a state round-trip, so a launch at 130% never flashes at
    // 100% first.
    applyZoom(readZoom());

    const onKeyDown = (event: KeyboardEvent) => {
      const intent = zoomIntent(event);
      if (intent === null) return;
      event.preventDefault();
      if (intent === "in") zoomIn();
      else if (intent === "out") zoomOut();
      else resetZoom();
    };

    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", onKeyDown, { capture: true });
  }, []);
}
