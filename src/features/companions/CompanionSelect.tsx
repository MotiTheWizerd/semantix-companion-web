// The composer's picker. Where a model list used to sit, the roster sits:
// you choose WHO you are talking to, and the companion brings its own voice
// and its own memory with it.

import { companionLabel, type Companion } from "./types";

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

  return (
    <select
      className={className}
      id={id}
      value={selected}
      disabled={disabled || companions.length === 0}
      onChange={(event) => onChange(event.target.value)}
    >
      {companions.length === 0 ? (
        <option value="">Loading companions…</option>
      ) : (
        companions.map((companion) => (
          <option key={companion.id} value={companion.id}>
            {companionLabel(companion)}
          </option>
        ))
      )}
    </select>
  );
}
