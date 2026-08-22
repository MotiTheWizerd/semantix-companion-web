export type ModelPreference =
  | { mode: "inherit" }
  | { mode: "test" }
  | { mode: "configured"; modelId: string };

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
  return preference.mode === "configured"
    ? `configured:${preference.modelId}`
    : preference.mode;
}

export function modelPreferenceFromValue(value: string): ModelPreference {
  if (value === "inherit") return { mode: "inherit" };
  if (value === "test") return { mode: "test" };
  if (value.startsWith("configured:")) {
    return { mode: "configured", modelId: value.slice("configured:".length) };
  }
  return { mode: "inherit" };
}
