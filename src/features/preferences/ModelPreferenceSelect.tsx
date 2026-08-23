// The one place a model gets picked. Two surfaces use it — a companion's own
// voice (which may inherit) and the user default (which may not) — so the
// option list and the "Default · <model>" wording are written once.

import type { ConfiguredModel } from "../models/configuredModels/types";
import {
  modelPreferenceFromValue,
  modelPreferenceValue,
  type ModelPreference,
  type UserPreferences,
} from "./types";

interface ModelPreferenceSelectProps {
  value: ModelPreference;
  configuredModels: ConfiguredModel[];
  /** Needed only to name what "Default" currently resolves to. */
  userPreferences?: UserPreferences;
  /** The user default itself cannot inherit — it is what gets inherited. */
  allowInherit?: boolean;
  disabled?: boolean;
  id?: string;
  className?: string;
  onChange: (preference: ModelPreference) => void;
}

/** What the user default resolves to right now, named for the Default row. */
function inheritedLabel(
  configuredModels: ConfiguredModel[],
  userPreferences: UserPreferences | undefined,
): string {
  const inherited = userPreferences?.defaultModel;
  if (!inherited || inherited.mode !== "configured") return "Test stream";
  return (
    configuredModels.find((model) => model.id === inherited.modelId)?.displayName ??
    "Unavailable model"
  );
}

export function ModelPreferenceSelect({
  value,
  configuredModels,
  userPreferences,
  allowInherit = false,
  disabled = false,
  id,
  className,
  onChange,
}: ModelPreferenceSelectProps) {
  return (
    <select
      className={className}
      id={id}
      value={modelPreferenceValue(value)}
      disabled={disabled}
      onChange={(event) => onChange(modelPreferenceFromValue(event.target.value))}
    >
      {allowInherit ? (
        <option value="inherit">
          Default · {inheritedLabel(configuredModels, userPreferences)}
        </option>
      ) : null}
      {/* The test stream is scaffolding, not a model, so it is never OFFERED.
          It still renders when something is already set to it — hiding the
          current value would make the control lie about what will answer. */}
      {value.mode === "test" ? <option value="test">Test stream</option> : null}
      {configuredModels.map((model) => (
        <option key={model.id} value={`configured:${model.id}`}>
          {model.displayName}
        </option>
      ))}
      {configuredModels.length === 0 && value.mode !== "test" ? (
        <option value="test" disabled>
          No models configured yet
        </option>
      ) : null}
    </select>
  );
}
