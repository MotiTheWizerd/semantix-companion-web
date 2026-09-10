import { Dropdown } from "../../components/Dropdown/Dropdown";
import type { Style } from "./types";
import styles from "./StyleSelector.module.css";

/** The one row that is not a style: the companion speaks in its own words. */
const PLAIN_KEY = "plain";

interface StyleRow {
  key: string;
  name: string;
  subtitle: string;
  styleId: string | null;
}

function exchangesLabel(count: number): string {
  if (count === 0) return "No exchanges yet";
  return count === 1 ? "1 exchange" : `${count} exchanges`;
}

function QuillMark({ plain }: { plain: boolean }) {
  return (
    <span
      className={`${styles.mark} ${plain ? styles.markPlain : ""}`}
      aria-hidden="true"
    >
      {plain ? (
        <svg viewBox="0 0 16 16" width="13" height="13">
          <path
            d="M3 8h10M3 4.5h7M3 11.5h5"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            fill="none"
          />
        </svg>
      ) : (
        <svg viewBox="0 0 16 16" width="13" height="13">
          <path
            d="M13.5 2.5c-4.5.3-8 3.2-9.3 7.6L3 13.5l.9-.2c4.4-1.3 7.3-4.8 7.6-9.3"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinejoin="round"
            fill="none"
          />
          <path
            d="M4.6 11.4 9.5 6.5"
            stroke="currentColor"
            strokeWidth="1.2"
            strokeLinecap="round"
          />
        </svg>
      )}
    </span>
  );
}

export interface StyleSelectorProps {
  /** The chosen style's id, or `null` for plain speech. */
  value: string | null;
  availableStyles: Style[];
  disabled?: boolean;
  id?: string;
  ariaLabel?: string;
  onChange: (styleId: string | null) => void;
}

/** Picks a voice from the style library on the shared Dropdown shell — the
 *  same trigger the model field wears, so the two sit as a pair in a form. */
export function StyleSelector({
  value,
  availableStyles,
  disabled = false,
  id,
  ariaLabel = "Style",
  onChange,
}: StyleSelectorProps) {
  const rows: StyleRow[] = [
    {
      key: PLAIN_KEY,
      name: "No style",
      subtitle: "Speaks plainly",
      styleId: null,
    },
    ...availableStyles.map((style) => ({
      key: style.id,
      name: style.name,
      subtitle: style.description?.trim() || exchangesLabel(style.exemplarCount),
      styleId: style.id,
    })),
  ];

  // A stored pick whose style is gone still shows — the control never lies
  // about what is set, it just stops offering the missing one as fresh.
  if (value !== null && !availableStyles.some((style) => style.id === value)) {
    rows.push({
      key: value,
      name: "Unavailable style",
      subtitle: "Removed from the library",
      styleId: value,
    });
  }

  const selected = rows.find((row) => row.styleId === value) ?? rows[0];

  return (
    <Dropdown
      items={rows}
      value={selected}
      onChange={(row) => onChange(row.styleId)}
      getItemKey={(row) => row.key}
      renderItem={(row) => (
        <div className={styles.item}>
          <QuillMark plain={row.styleId === null} />
          <span className={styles.copy}>
            <span className={styles.name}>{row.name}</span>
            <span className={styles.subtitle}>{row.subtitle}</span>
          </span>
        </div>
      )}
      renderTrigger={() => (
        <span className={styles.selected}>
          <QuillMark plain={selected.styleId === null} />
          <span className={styles.selectedCopy}>
            <span className={styles.selectedName}>{selected.name}</span>
            <span className={styles.selectedSubtitle}>{selected.subtitle}</span>
          </span>
        </span>
      )}
      placeholder="Select style"
      disabled={disabled}
      id={id}
      ariaLabel={ariaLabel}
      menuLabel="Style library"
      className={styles.dropdown}
      triggerClassName={styles.formTrigger}
      menuClassName={styles.menu}
      searchable={availableStyles.length > 5}
      searchPlaceholder="Search styles..."
      emptyMessage="No style matches"
      getSearchText={(row) => `${row.name} ${row.subtitle}`}
      maxVisibleItems={6}
    />
  );
}
