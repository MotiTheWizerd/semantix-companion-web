/**
 * A companion's face at chrome size. One component behind every surface
 * that shows who is speaking or being spoken to: the sidebar brand, the
 * presence line's breathing orb, the tab faces, both faces of the picker,
 * the byline.
 *
 * Three looks, in order of preference:
 *   1. `src`  — the companion's own picture, cropped to a circle.
 *   2. `name` — no picture yet: a lettermark, the first letter of the name
 *      on a gradient tile whose hue is the name's own (s571: "the default
 *      avatar should be the first letter of their name, like Google's, but
 *      more modern"). The same name always gets the same colour, on every
 *      surface, so a companion is recognisable before it has a face.
 *   3. neither — the Semantix mark. This is the BRAND, not a companion:
 *      the sidebar wears it; nothing that stands for a companion should
 *      reach it any more.
 */

import { useId } from "react";

/** Eight hues from the sky's own family — violet through blue and cyan to
 *  teal, then pink, coral and amber — spaced so neighbours in a roster read
 *  as different at 16px. A curated ring, not the full wheel: any-hue
 *  hashing lands on muddy yellows and greens that fight the glass. */
const LETTERMARK_HUES = [292, 268, 245, 220, 195, 170, 350, 25, 60];

/** A stable, cheap hash — the same name is the same colour on every run. */
function hueFor(name: string): number {
  let hash = 5381;
  for (const char of name) hash = ((hash << 5) + hash + char.codePointAt(0)!) | 0;
  return LETTERMARK_HUES[Math.abs(hash) % LETTERMARK_HUES.length];
}

/** The first letter as the person sees it — one grapheme, so a name that
 *  starts with an emoji or a two-unit character keeps it whole. */
function initialOf(name: string): string {
  return [...name.trim()][0]?.toUpperCase() ?? "";
}

export function CompanionMark({ src, name }: { src?: string | null; name?: string | null }) {
  const gradientId = useId();
  const initial = !src && name ? initialOf(name) : "";

  if (initial) {
    return (
      <span className="companion-mark" aria-hidden="true">
        <svg
          className="companion-mark__initial"
          viewBox="0 0 32 32"
          style={{ "--mark-hue": hueFor(name!.trim()) } as React.CSSProperties}
        >
          <defs>
            <linearGradient id={gradientId} x1="0" y1="0" x2="1" y2="1">
              <stop offset="0" className="companion-mark__initial-light" />
              <stop offset="1" className="companion-mark__initial-deep" />
            </linearGradient>
          </defs>
          <circle cx="16" cy="16" r="16" fill={`url(#${gradientId})`} />
          <circle cx="16" cy="16" r="15.25" className="companion-mark__initial-rim" />
          <text
            x="16"
            y="16.5"
            textAnchor="middle"
            dominantBaseline="central"
            className="companion-mark__initial-letter"
          >
            {initial}
          </text>
        </svg>
      </span>
    );
  }

  return (
    <span className="companion-mark" aria-hidden="true">
      <img
        className={`companion-mark__img${src ? " companion-mark__img--avatar" : ""}`}
        src={src || "/logo-mark.png"}
        alt=""
        draggable={false}
      />
    </span>
  );
}
