/**
 * The Companion mark at chrome size — the logo, not the old CSS orb. One
 * component behind four surfaces: the sidebar brand, the presence line's
 * breathing orb, and both faces of the companion picker. The image carries
 * the animation the core used to, so the presence line still breathes.
 *
 * A companion with a picture of its own wears that instead. The mark is the
 * FALLBACK, not the default — pass `src` and this shows a face; pass nothing
 * and every surface looks exactly as it did before avatars existed.
 */
export function CompanionMark({ src }: { src?: string | null }) {
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
