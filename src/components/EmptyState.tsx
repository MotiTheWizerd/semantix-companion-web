import { LogoMark } from "./LogoMark";

/** Purely presentational welcome hero for a fresh conversation. */
export function EmptyState() {
  return (
    <section className="chat-empty-state" aria-labelledby="welcome-title">
      <div className="chat-empty-state__glow" aria-hidden="true" />
      <div className="chat-empty-state__logo">
        <LogoMark size={96} />
      </div>
      <h1 id="welcome-title" className="chat-empty-state__wordmark">
        Semantix
      </h1>
    </section>
  );
}
