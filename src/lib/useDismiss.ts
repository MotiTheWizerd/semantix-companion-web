import { useEffect, useRef, type RefObject } from "react";

interface UseDismissOptions {
  /** Bind listeners only while the surface is open. */
  open: boolean;
  /** Container ref; a mousedown outside it dismisses. */
  ref: RefObject<HTMLElement | null>;
  /** Called on click-outside or Escape. */
  onDismiss: () => void;
  /** Also dismiss on Escape. Default true. */
  escape?: boolean;
  /**
   * CSS selector for elements that must NOT count as "outside" — a click whose
   * target (or any ancestor) matches is ignored instead of dismissing. Use for
   * portaled content that lives outside the container ref.
   */
  ignore?: string;
}

/**
 * Dismiss-on-outside-click (+ optional Escape) for popovers, dropdowns, and
 * menus. Migrated from the studio's canonical hook (semantix-indexer/studio).
 *
 * The latest `onDismiss` is read through a ref, so listeners bind once per
 * open instead of re-binding on every render.
 */
export function useDismiss({
  open,
  ref,
  onDismiss,
  escape = true,
  ignore,
}: UseDismissOptions): void {
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;

  useEffect(() => {
    if (!open) return;

    const handlePointer = (event: MouseEvent) => {
      const target = event.target as Element | null;
      if (ignore && target?.closest(ignore)) return;
      if (ref.current && !ref.current.contains(event.target as Node)) {
        onDismissRef.current();
      }
    };
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onDismissRef.current();
    };

    document.addEventListener("mousedown", handlePointer);
    if (escape) document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("mousedown", handlePointer);
      if (escape) document.removeEventListener("keydown", handleKey);
    };
  }, [open, ref, escape, ignore]);
}
