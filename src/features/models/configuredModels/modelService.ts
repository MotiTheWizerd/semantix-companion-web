import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  ConfiguredModel,
  CreateConfiguredModelInput,
  ModelChangedEvent,
  UpdateConfiguredModelInput,
} from "./types";

const MODELS_CHANGED_EVENT = "models://changed";

export function listConfiguredModels(): Promise<ConfiguredModel[]> {
  return invoke<ConfiguredModel[]>("list_configured_models");
}

export function createConfiguredModel(
  input: CreateConfiguredModelInput,
): Promise<ConfiguredModel> {
  return invoke<ConfiguredModel>("create_configured_model", { input });
}

export function updateConfiguredModel(
  input: UpdateConfiguredModelInput,
): Promise<ConfiguredModel> {
  return invoke<ConfiguredModel>("update_configured_model", { input });
}

export function deleteConfiguredModel(modelId: string): Promise<void> {
  return invoke<void>("delete_configured_model", { modelId });
}

export function onModelsChanged(
  handler: (event: ModelChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<ModelChangedEvent>(MODELS_CHANGED_EVENT, ({ payload }) => {
    handler(payload);
  });
}
