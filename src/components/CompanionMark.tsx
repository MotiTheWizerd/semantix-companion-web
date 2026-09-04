/**
 * The Companion mark at chrome size — the logo, not the old CSS orb. One
 * component behind four surfaces: the sidebar brand, the presence line's
 * breathing orb, and both faces of the companion picker. The image carries
 * the animation the core used to, so the presence line still breathes.
 */
export function CompanionMark() {
  return (
    <span className="companion-mark" aria-hidden="true">
      <img className="companion-mark__img" src="/logo-mark.png" alt="" draggable={false} />
    </span>
  );
}
