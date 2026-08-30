import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  CreateStyleInput,
  HarvestResult,
  HarvestStyleInput,
  Style,
  StyleChangedEvent,
  StyleExemplar,
  StyleSourceInspection,
  UpdateStyleInput,
} from "./types";

const STYLES_CHANGED_EVENT = "styles://changed";

export function listStyles(): Promise<Style[]> {
  return invoke<Style[]>("list_styles");
}

export function getStyleExemplars(styleId: string): Promise<StyleExemplar[]> {
  return invoke<StyleExemplar[]>("get_style_exemplars", { styleId });
}

export function createStyle(input: CreateStyleInput): Promise<Style> {
  return invoke<Style>("create_style", { input });
}

export function updateStyle(input: UpdateStyleInput): Promise<Style> {
  return invoke<Style>("update_style", { input });
}

export function deleteStyle(styleId: string): Promise<void> {
  return invoke<void>("delete_style", { styleId });
}

export function inspectStyleSource(path: string): Promise<StyleSourceInspection> {
  return invoke<StyleSourceInspection>("inspect_style_source", { path });
}

export function harvestStyleExemplars(input: HarvestStyleInput): Promise<HarvestResult> {
  return invoke<HarvestResult>("harvest_style_exemplars", { input });
}

export function onStylesChanged(
  handler: (event: StyleChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<StyleChangedEvent>(STYLES_CHANGED_EVENT, ({ payload }) => {
    handler(payload);
  });
}

/** Fold one change event into the library, kept in creation order — the same
 *  order Rust lists in. */
export function reconcileStyleEvent(styles: Style[], event: StyleChangedEvent): Style[] {
  if (event.kind === "deleted") {
    return styles.filter((style) => style.id !== event.styleId);
  }
  return [...styles.filter((style) => style.id !== event.style.id), event.style].sort(
    (left, right) =>
      left.createdAt !== right.createdAt
        ? left.createdAt - right.createdAt
        : left.id.localeCompare(right.id),
  );
}
