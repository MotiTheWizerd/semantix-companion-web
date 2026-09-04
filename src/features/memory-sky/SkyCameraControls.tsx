// The camera, by hand — a glass pill of five: closer, farther, swing one
// way, swing the other, and the whole mind. A tap moves one beat; a hold
// keeps moving until it is let go. It swings around whatever the sky is
// looking at, so it never needs a node picked. The arrow keys, + / − and
// Home do the same from the keyboard (bound in MemorySkyView).

import { useRef, type KeyboardEvent, type PointerEvent, type ReactElement } from "react";

import type { MemorySky } from "./engine/MemorySky";

/** A press held past this is a steer, not a tap. */
const HOLD_AFTER_MS = 200;
/** One tap, in degrees around the target / as a distance factor. */
export const TAP_DEG = 28;
export const TAP_ZOOM = 0.72;
/** A hold, per second. */
const HOLD_DEG_PER_S = 70;
const HOLD_ZOOM_PER_S = 0.45;

interface Move {
  label: string;
  tap: (sky: MemorySky) => void;
  hold: (sky: MemorySky) => void;
  icon: () => ReactElement;
}

const MOVES: Move[] = [
  {
    label: "Closer  (+)",
    tap: (sky) => sky.nudgeZoom(TAP_ZOOM),
    hold: (sky) => sky.steer(0, 0, HOLD_ZOOM_PER_S),
    icon: () => (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M5 12h14M12 5v14" />
      </svg>
    ),
  },
  {
    label: "Farther  (−)",
    tap: (sky) => sky.nudgeZoom(1 / TAP_ZOOM),
    hold: (sky) => sky.steer(0, 0, 1 / HOLD_ZOOM_PER_S),
    icon: () => (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M5 12h14" />
      </svg>
    ),
  },
  {
    label: "Swing left  (←)",
    tap: (sky) => sky.nudgeOrbit(-TAP_DEG, 0),
    hold: (sky) => sky.steer(-HOLD_DEG_PER_S, 0),
    icon: () => (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
        <path d="M3 3v5h5" />
      </svg>
    ),
  },
  {
    label: "Swing right  (→)",
    tap: (sky) => sky.nudgeOrbit(TAP_DEG, 0),
    hold: (sky) => sky.steer(HOLD_DEG_PER_S, 0),
    icon: () => (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" />
        <path d="M21 3v5h-5" />
      </svg>
    ),
  },
];

interface SkyCameraControlsProps {
  /** The engine, read at press time — it is built after the first render. */
  sky: () => MemorySky | null;
}

export function SkyCameraControls({ sky }: SkyCameraControlsProps) {
  const holdTimer = useRef<number | null>(null);
  const isHolding = useRef(false);

  const clearHold = () => {
    if (holdTimer.current !== null) window.clearTimeout(holdTimer.current);
    holdTimer.current = null;
  };

  const press = (move: Move) => (event: PointerEvent<HTMLButtonElement>) => {
    if (event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    clearHold();
    isHolding.current = false;
    holdTimer.current = window.setTimeout(() => {
      holdTimer.current = null;
      const engine = sky();
      if (!engine) return;
      isHolding.current = true;
      move.hold(engine);
    }, HOLD_AFTER_MS);
  };

  /** Let go: a hold eases out; a press that never became one is a tap. */
  const release = (move: Move, tapped: boolean) => () => {
    const wasPending = holdTimer.current !== null;
    clearHold();
    const engine = sky();
    if (!engine) return;
    if (isHolding.current) engine.steer(0, 0);
    else if (wasPending && tapped) move.tap(engine);
    isHolding.current = false;
  };

  const key = (move: Move) => (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    const engine = sky();
    if (engine) move.tap(engine);
  };

  return (
    <div className="memory-sky__camera" role="group" aria-label="Camera">
      {MOVES.map((move, index) => (
        <button
          key={move.label}
          type="button"
          className={index === 2 ? "is-after-gap" : undefined}
          title={move.label}
          aria-label={move.label}
          onPointerDown={press(move)}
          onPointerUp={release(move, true)}
          onPointerCancel={release(move, false)}
          onLostPointerCapture={release(move, false)}
          onKeyDown={key(move)}
          onContextMenu={(event) => event.preventDefault()}
        >
          <move.icon />
        </button>
      ))}
      <button
        type="button"
        className="is-after-gap"
        title="The whole mind  (Home)"
        aria-label="The whole mind"
        onClick={() => sky()?.home()}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="12" cy="12" r="3" />
          <path d="M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2" />
        </svg>
      </button>
    </div>
  );
}
