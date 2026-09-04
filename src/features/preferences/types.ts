export type ModelPreference =
  | { mode: "inherit" }
  | { mode: "test" }
  | { mode: "configured"; modelId: string }
  // A Claude Code model (SDK alias like "opus") — selectable now, wired into
  // chat in the integration round.
  | { mode: "claude_code"; modelId: string };

export interface UserPreferences {
  defaultModel: Exclude<ModelPreference, { mode: "inherit" }>;
  /** What to call the person using this install. `null` until they say so —
   *  the interface falls back to "You" rather than inventing a name. */
  displayName: string | null;
  updatedAt: number;
}

/** A PATCH: an omitted field is left as it was. Sending `displayName: ""`
 *  clears the name; omitting it entirely leaves the stored one alone. */
export interface UpdateUserPreferencesInput {
  defaultModel?: Exclude<ModelPreference, { mode: "inherit" }>;
  displayName?: string;
}

/** The name the interface actually shows. One function so the sidebar, the
 *  settings form and anything later cannot drift on what "unset" looks like. */
export function userDisplayName(preferences: UserPreferences): string {
  return preferences.displayName?.trim() || "You";
}

/** The avatar letter. Derived from whatever is displayed, so it can never
 *  disagree with the name beside it. */
export function userInitial(preferences: UserPreferences): string {
  return [...userDisplayName(preferences)][0]?.toUpperCase() ?? "Y";
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
