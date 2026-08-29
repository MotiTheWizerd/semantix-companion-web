// The composer's picker. Where a model list used to sit, the roster sits:
// you choose WHO you are talking to, and the companion brings its own voice
// and its own memory with it.

import { CompanionMark } from "../../components/CompanionMark";
import { Dropdown } from "../../components/Dropdown/Dropdown";
import { companionLabel, type Companion } from "./types";
import styles from "./CompanionSelect.module.css";

interface CompanionSelectProps {
  companions: Companion[];
  /** null = nothing picked yet; the built-in companion answers. */
  value: string | null;
  disabled?: boolean;
  id?: string;
  className?: string;
  onChange: (companionId: string) => void;
}

export function CompanionSelect({
  companions,
  value,
  disabled = false,
  id,
  className,
  onChange,
}: CompanionSelectProps) {
  const builtIn = companions.find((companion) => companion.isBuiltIn);
  // An unpicked thread already answers to the built-in companion in Rust;
  // showing it selected here keeps the control honest about what will happen.
  const selected = value ?? builtIn?.id ?? "";
  const selectedCompanion =
    companions.find((companion) => companion.id === selected) ?? builtIn ?? null;

  return (
    <Dropdown
      items={companions}
      value={selectedCompanion}
      onChange={(companion) => onChange(companion.id)}
      getItemKey={(companion) => companion.id}
      renderItem={(companion) => (
        <span className={styles.companionItem}>
          <span className={styles.companionMark}>
            <CompanionMark />
          </span>
          <span className={styles.companionCopy}>
            <span className={styles.companionName}>
              {companionLabel(companion)}
            </span>
            <span className={styles.companionDetail}>
              {companionDetail(companion)}
            </span>
          </span>
        </span>
      )}
      renderTrigger={() => (
        <span className={styles.selectedCompanion}>
          <span className={`${styles.companionMark} ${styles.selectedMark}`}>
            <CompanionMark />
          </span>
          <span className={styles.selectedName}>
            {selectedCompanion
              ? companionLabel(selectedCompanion)
              : "Loading companions…"}
          </span>
        </span>
      )}
      placeholder="Loading companions…"
      disabled={disabled || companions.length === 0}
      id={id}
      ariaLabel="Companion"
      menuLabel="Available companions"
      className={`${styles.dropdown} ${className ?? ""}`}
      triggerClassName={styles.trigger}
      menuClassName={styles.menu}
      direction="up"
      searchable
      searchPlaceholder="Search companions..."
      emptyMessage="No companions available"
      getSearchText={companionLabel}
      menuHeader={
        <div className={styles.menuHeading}>
          <span>Conversation partner</span>
          <strong>Choose a companion</strong>
        </div>
      }
    />
  );
}

function companionDetail(companion: Companion): string {
  const kind = companion.isBuiltIn ? "Primary companion" : "Companion";
  const count = companion.workspaces.length;
  const workspaces = `${count} workspace${count === 1 ? "" : "s"}`;
  return `${kind} · ${workspaces}`;
}
