import { Dropdown } from "../Dropdown/Dropdown";
import styles from "./FormSelect.module.css";

export interface FormSelectOption {
  /** Stable identity; what `onChange` hands back. */
  value: string;
  label: string;
  /** Muted trailing text — a key hint, a count, a provider. */
  detail?: string;
}

export interface FormSelectProps {
  options: FormSelectOption[];
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  id?: string;
  ariaLabel?: string;
  menuLabel?: string;
  searchable?: boolean;
  searchPlaceholder?: string;
  emptyMessage?: string;
}

/** A single-line pick on the shared Dropdown shell, sized to sit beside a
 *  text input in a settings form. The native `<select>` it replaces was the
 *  one control in those forms that did not wear the theme. */
export function FormSelect({
  options,
  value,
  onChange,
  placeholder = "Select…",
  disabled = false,
  id,
  ariaLabel,
  menuLabel = "Options",
  searchable = false,
  searchPlaceholder = "Search…",
  emptyMessage = "No match",
}: FormSelectProps) {
  const selected = options.find((option) => option.value === value) ?? null;

  const renderRow = (option: FormSelectOption) => (
    <span className={styles.row}>
      <span className={styles.label}>{option.label}</span>
      {option.detail ? <span className={styles.detail}>{option.detail}</span> : null}
    </span>
  );

  return (
    <Dropdown
      items={options}
      value={selected}
      onChange={(option) => onChange(option.value)}
      getItemKey={(option) => option.value}
      renderItem={renderRow}
      renderTrigger={() =>
        selected ? (
          renderRow(selected)
        ) : (
          <span className={styles.placeholder}>{placeholder}</span>
        )
      }
      placeholder={placeholder}
      disabled={disabled}
      id={id}
      ariaLabel={ariaLabel}
      menuLabel={menuLabel}
      className={styles.dropdown}
      triggerClassName={styles.trigger}
      menuClassName={styles.menu}
      searchable={searchable}
      searchPlaceholder={searchPlaceholder}
      emptyMessage={emptyMessage}
      getSearchText={(option) => `${option.label} ${option.detail ?? ""}`}
      maxVisibleItems={7}
    />
  );
}
