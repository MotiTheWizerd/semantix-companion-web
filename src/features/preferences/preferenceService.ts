import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  UpdateUserPreferencesInput,
  UserPreferences,
  UserPreferencesChangedEvent,
} from "./types";

const USER_PREFERENCES_CHANGED_EVENT = "preferences://changed";

export function getUserPreferences(): Promise<UserPreferences> {
  return invoke<UserPreferences>("get_user_preferences");
}

export function updateUserPreferences(
  input: UpdateUserPreferencesInput,
): Promise<UserPreferences> {
  return invoke<UserPreferences>("update_user_preferences", { input });
}

export function onUserPreferencesChanged(
  handler: (event: UserPreferencesChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<UserPreferencesChangedEvent>(USER_PREFERENCES_CHANGED_EVENT, ({ payload }) => {
    handler(payload);
  });
}
