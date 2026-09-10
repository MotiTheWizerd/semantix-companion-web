/** A tooltip — the app's own, not the browser's.
 *
 * Wraps one element and says something about it on hover or focus, after a
 * short delay, in the same glass the menus are made of. Portaled to <body>
 * and positioned fixed, so no scrolling or overflow-hidden ancestor can clip
 * it; placed on the side the caller asks for and flipped to the other side
 * when the viewport has no room there.
 *
 *   <Tooltip content={conversation.title} placement="right" whenTruncated>
 *     <button className="conversation-list__item">{conversation.title}</button>
 *   </Tooltip>
 *
 * `whenTruncated` is for text that is cut with an ellipsis: the tooltip shows
 * only if the anchor (or anything inside it) actually overflows, so a name
 * that fits never gets a redundant copy of itself floating beside it.
 *
 * Hovering from one tooltipped thing to the next within a beat opens the next
 * one at once — the delay guards against tooltips on a passing cursor, not
 * against a person who is clearly reading labels. */

import {
  Children,
  cloneElement,
  isValidElement,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
  type Ref,
} from "react";
import { createPortal } from "react-dom";

import styles from "./Tooltip.module.css";

export type TooltipPlacement = "top" | "bottom" | "left" | "right";

interface AnchorProps {
  ref?: Ref<HTMLElement>;
  onMouseEnter?: (event: MouseEvent<HTMLElement>) => void;
  onMouseLeave?: (event: MouseEvent<HTMLElement>) => void;
  onFocus?: (event: FocusEvent<HTMLElement>) => void;
  onBlur?: (event: FocusEvent<HTMLElement>) => void;
  "aria-describedby"?: string;
}

interface TooltipProps {
  content: ReactNode;
  /** Which side of the anchor to sit on. Flips when that side has no room. */
  placement?: TooltipPlacement;
  /** Show only when the anchor's text is actually cut short. */
  whenTruncated?: boolean;
  /** How long the cursor must rest before the tooltip appears. */
  delayMs?: number;
  children: ReactElement<AnchorProps>;
}

/** Gap between the anchor and the tooltip. */
const OFFSET_PX = 8;
/** The tooltip never comes closer than this to the viewport's edge. */
const VIEWPORT_MARGIN_PX = 8;
/** After one tooltip hides, the next opens without its delay for this long. */
const WARM_MS = 300;
const DEFAULT_DELAY_MS = 450;

/** When the last tooltip anywhere hid — shared, so moving across a row of
 *  labels reads as one gesture. */
let lastHiddenAt = 0;

interface Coords {
  left: number;
  top: number;
  placement: TooltipPlacement;
}

/** Whether the anchor, or anything inside it, has text cut short. Checked on
 *  the descendants too because the ellipsis usually lives on an inner span —
 *  and MUST for a <button>: a button never reports overflow, its scrollWidth
 *  stays at clientWidth however long the text, so an ellipsis on the button
 *  itself is invisible to this check. Put it on a span inside. */
function isTruncated(root: HTMLElement): boolean {
  if (root.scrollWidth > root.clientWidth + 1) return true;
  for (const element of Array.from(root.querySelectorAll<HTMLElement>("*"))) {
    if (element.scrollWidth > element.clientWidth + 1) return true;
  }
  // The layout-independent answer: the text's own laid-out width against
  // the box it was given. Clipping never shrinks a text node's rects, so
  // this sees a cut even where the boxes above refuse to report one.
  const range = document.createRange();
  range.selectNodeContents(root);
  const textWidth = range.getBoundingClientRect().width;
  return textWidth > root.getBoundingClientRect().width + 1;
}

function opposite(placement: TooltipPlacement): TooltipPlacement {
  switch (placement) {
    case "top":
      return "bottom";
    case "bottom":
      return "top";
    case "left":
      return "right";
    case "right":
      return "left";
  }
}

/** Where the tip sits for a placement — null when that side has no room. */
function coordsFor(
  placement: TooltipPlacement,
  anchor: DOMRect,
  tip: DOMRect,
): Coords | null {
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  let left: number;
  let top: number;
  switch (placement) {
    case "top":
      left = anchor.left + anchor.width / 2 - tip.width / 2;
      top = anchor.top - OFFSET_PX - tip.height;
      if (top < VIEWPORT_MARGIN_PX) return null;
      break;
    case "bottom":
      left = anchor.left + anchor.width / 2 - tip.width / 2;
      top = anchor.bottom + OFFSET_PX;
      if (top + tip.height > viewportHeight - VIEWPORT_MARGIN_PX) return null;
      break;
    case "left":
      left = anchor.left - OFFSET_PX - tip.width;
      top = anchor.top + anchor.height / 2 - tip.height / 2;
      if (left < VIEWPORT_MARGIN_PX) return null;
      break;
    case "right":
      left = anchor.right + OFFSET_PX;
      top = anchor.top + anchor.height / 2 - tip.height / 2;
      if (left + tip.width > viewportWidth - VIEWPORT_MARGIN_PX) return null;
      break;
  }
  // Slide along the anchor's edge to stay inside the viewport.
  left = Math.min(
    Math.max(left, VIEWPORT_MARGIN_PX),
    viewportWidth - VIEWPORT_MARGIN_PX - tip.width,
  );
  top = Math.min(
    Math.max(top, VIEWPORT_MARGIN_PX),
    viewportHeight - VIEWPORT_MARGIN_PX - tip.height,
  );
  return { left, top, placement };
}

export function Tooltip({
  content,
  placement = "top",
  whenTruncated = false,
  delayMs = DEFAULT_DELAY_MS,
  children,
}: TooltipProps) {
  const child = Children.only(children);
  if (!isValidElement<AnchorProps>(child)) {
    throw new Error("Tooltip wraps exactly one element.");
  }
  const tooltipId = useId();
  const anchorRef = useRef<HTMLElement | null>(null);
  const tipRef = useRef<HTMLDivElement | null>(null);
  const timerRef = useRef<number | null>(null);
  const [isOpen, setIsOpen] = useState(false);
  const [coords, setCoords] = useState<Coords | null>(null);

  const clearTimer = () => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  };

  const show = useCallback(() => {
    const anchor = anchorRef.current;
    if (!anchor) return;
    if (whenTruncated && !isTruncated(anchor)) return;
    clearTimer();
    const warm = Date.now() - lastHiddenAt < WARM_MS;
    if (warm) {
      setIsOpen(true);
      return;
    }
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null;
      setIsOpen(true);
    }, delayMs);
  }, [delayMs, whenTruncated]);

  const hide = useCallback(() => {
    clearTimer();
    setIsOpen((open) => {
      if (open) lastHiddenAt = Date.now();
      return false;
    });
    setCoords(null);
  }, []);

  // The anchor's ref is ours AND the child's, if it had one.
  const setAnchor = useCallback(
    (node: HTMLElement | null) => {
      anchorRef.current = node;
      const childRef = child.props.ref;
      if (typeof childRef === "function") {
        childRef(node);
      } else if (childRef && typeof childRef === "object") {
        (childRef as { current: HTMLElement | null }).current = node;
      }
    },
    [child.props.ref],
  );

  // Place it once it exists, measured, in the frame before it is seen.
  useLayoutEffect(() => {
    if (!isOpen) return;
    const anchor = anchorRef.current;
    const tip = tipRef.current;
    if (!anchor || !tip) return;
    const anchorRect = anchor.getBoundingClientRect();
    const tipRect = tip.getBoundingClientRect();
    setCoords(
      coordsFor(placement, anchorRect, tipRect) ??
        coordsFor(opposite(placement), anchorRect, tipRect) ??
        coordsFor("bottom", anchorRect, tipRect) ??
        coordsFor("top", anchorRect, tipRect) ?? {
          left: VIEWPORT_MARGIN_PX,
          top: VIEWPORT_MARGIN_PX,
          placement,
        },
    );
  }, [isOpen, placement, content]);

  // Escape, a scroll, a click, a resize: the tooltip stands down. It is a
  // label, and a label that chases the page is a distraction.
  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") hide();
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("scroll", hide, { capture: true, passive: true });
    window.addEventListener("resize", hide);
    window.addEventListener("pointerdown", hide, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("scroll", hide, { capture: true });
      window.removeEventListener("resize", hide);
      window.removeEventListener("pointerdown", hide, { capture: true });
    };
  }, [isOpen, hide]);

  useEffect(() => clearTimer, []);

  const anchor = cloneElement(child, {
    ref: setAnchor,
    onMouseEnter: (event: MouseEvent<HTMLElement>) => {
      child.props.onMouseEnter?.(event);
      show();
    },
    onMouseLeave: (event: MouseEvent<HTMLElement>) => {
      child.props.onMouseLeave?.(event);
      hide();
    },
    onFocus: (event: FocusEvent<HTMLElement>) => {
      child.props.onFocus?.(event);
      show();
    },
    onBlur: (event: FocusEvent<HTMLElement>) => {
      child.props.onBlur?.(event);
      hide();
    },
    "aria-describedby": isOpen ? tooltipId : child.props["aria-describedby"],
  });

  return (
    <>
      {anchor}
      {isOpen
        ? createPortal(
            <div
              ref={tipRef}
              id={tooltipId}
              role="tooltip"
              className={styles.tooltip}
              data-placement={coords?.placement ?? placement}
              style={{
                position: "fixed",
                left: coords?.left ?? 0,
                top: coords?.top ?? 0,
                // Pre-measurement frame: mounted so it can be measured,
                // invisible so it is never seen at the wrong place.
                visibility: coords ? "visible" : "hidden",
              }}
            >
              {content}
            </div>,
            document.body,
          )
        : null}
    </>
  );
}
