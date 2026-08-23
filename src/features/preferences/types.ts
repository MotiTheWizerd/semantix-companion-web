export type ModelPreference =
  | { mode: "inherit" }
  | { mode: "test" }
  | { mode: "configured"; modelId: string }
  // A Claude Code model (SDK alias like "opus") — selectable now, wired into
  // chat in the integration round.
  | { mode: "claude_code"; modelId: string };

export interface UserPreferences {
  defaultModel: Exclude<ModelPreference, { mode: "inherit" }>;
  updatedAt: number;
}

export interface UpdateUserPreferencesInput {
  defaultModel: Exclude<ModelPreference, { mode: "inherit" }>;
}

export type UserPreferencesChangedEvent = {
  kind: "updated";
  preferences: UserPreferences;
};

export function modelPreferenceValue(preference: ModelPreference): string {
  if (preference.mode === "configured") return `configured:${preference.modelId}`;
  if (preference.mode === "claude_code") return `claude_code:${preference.modelId}`;
  return preference.mode;
}

export function modelPreferenceFromValue(value: string): ModelPreference {
  if (value === "inherit") return { mode: "inherit" };
  if (value === "test") return { mode: "test" };
  if (value.startsWith("configured:")) {
    return { mode: "configured", modelId: value.slice("configured:".length) };
  }
  if (value.startsWith("claude_code:")) {
    return { mode: "claude_code", modelId: value.slice("claude_code:".length) };
  }
  return { mode: "inherit" };
}
