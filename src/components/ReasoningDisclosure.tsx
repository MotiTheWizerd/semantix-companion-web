import { useId, useState } from "react";

import styles from "./ReasoningDisclosure.module.css";

interface ReasoningDisclosureProps {
  reasoning: string;
  isStreaming: boolean;
}

function ThinkingIcon() {
  return (
    <svg viewBox="0 0 18 18" aria-hidden="true">
      <path d="M9 2.25a5.25 5.25 0 0 0-3.16 9.45c.45.34.73.84.73 1.4v.15h4.86v-.15c0-.56.28-1.06.73-1.4A5.25 5.25 0 0 0 9 2.25Z" />
      <path d="M7 15.25h4M7.75 13.25v2M10.25 13.25v2" />
    </svg>
  );
}

function ChevronIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="m5.5 6.5 2.5 2.5 2.5-2.5" />
    </svg>
  );
}

/** A provider-neutral view of explicitly exposed reasoning and progress text
 * reclassified when a tool begins. Hidden reasoning is never synthesized. */
export function ReasoningDisclosure({
  reasoning,
  isStreaming,
}: ReasoningDisclosureProps) {
  const [isOpen, setIsOpen] = useState(false);
  const panelId = useId();
  const hasReasoning = reasoning.length > 0;
  const label = isStreaming ? "Thinking…" : "Thought process";

  return (
    <div className={`${styles.root} ${isOpen ? styles.open : ""}`}>
      <button
        type="button"
        className={styles.trigger}
        aria-expanded={isOpen}
        aria-controls={panelId}
        onClick={() => setIsOpen((open) => !open)}
      >
        <span className={styles.thinkingIcon}>
          <ThinkingIcon />
        </span>
        <span>{label}</span>
        {isStreaming ? <span className={styles.pulse} aria-hidden="true" /> : null}
        <span className={styles.chevron}>
          <ChevronIcon />
        </span>
      </button>
      {isOpen ? (
        <div id={panelId} className={styles.panel}>
          {hasReasoning ? (
            <div className={styles.content}>{reasoning}</div>
          ) : (
            <div className={styles.waiting}>Waiting for provider-supplied thoughts…</div>
          )}
        </div>
      ) : null}
    </div>
  );
}
