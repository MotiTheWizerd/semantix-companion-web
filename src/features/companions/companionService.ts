import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  Companion,
  CompanionChangedEvent,
  CreateCompanionInput,
  UpdateCompanionInput,
} from "./types";

const COMPANIONS_CHANGED_EVENT = "companions://changed";

export function listCompanions(): Promise<Companion[]> {
  return invoke<Companion[]>("list_companions");
}

export function createCompanion(input: CreateCompanionInput): Promise<Companion> {
  return invoke<Companion>("create_companion", { input });
}

export function updateCompanion(input: UpdateCompanionInput): Promise<Companion> {
  return invoke<Companion>("update_companion", { input });
}

export function deleteCompanion(companionId: string): Promise<void> {
  return invoke<void>("delete_companion", { companionId });
}

export function onCompanionsChanged(
  handler: (event: CompanionChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<CompanionChangedEvent>(COMPANIONS_CHANGED_EVENT, ({ payload }) => {
    handler(payload);
  });
}

/** Fold one change event into a roster, keeping the built-in companion first
 *  and the rest in creation order — the same order Rust lists them in. */
export function reconcileCompanionEvent(
  companions: Companion[],
  event: CompanionChangedEvent,
): Companion[] {
  if (event.kind === "deleted") {
    return companions.filter((companion) => companion.id !== event.companionId);
  }

  return [
    ...companions.filter((companion) => companion.id !== event.companion.id),
    event.companion,
  ].sort((left, right) => {
    if (left.isBuiltIn !== right.isBuiltIn) return left.isBuiltIn ? -1 : 1;
    if (left.createdAt !== right.createdAt) return left.createdAt - right.createdAt;
    return left.id.localeCompare(right.id);
  });
}
