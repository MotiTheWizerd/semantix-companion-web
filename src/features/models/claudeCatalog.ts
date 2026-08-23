// The Claude Code model catalog. STATIC for now — the companion has no Claude
// sidecar yet, so this is the picker's offer list until the integration round
// replaces it with the SDK's live `supportedModels()` (the way the studio
// fetches it over its :3001 sidecar).
//
// Ids are the SDK ALIASES ("opus", not a dated model id) so a stored pick
// keeps meaning "the current Opus" once the wiring lands.

export interface ClaudeModel {
  /** SDK model alias, stored as the preference's modelId. */
  id: string;
  /** Display name for the picker. */
  label: string;
  /** One-line blurb rendered under the name. */
  description?: string;
}

export const CLAUDE_MODELS: ClaudeModel[] = [
  { id: "opus", label: "Opus", description: "Most capable" },
  { id: "sonnet", label: "Sonnet", description: "Best for everyday tasks" },
  { id: "haiku", label: "Haiku", description: "Fastest" },
];

/** Name a stored Claude pick, falling back to the raw alias for an id the
 *  catalog no longer lists — the control must not lie about what is set. */
export function claudeModelLabel(modelId: string): string {
  return CLAUDE_MODELS.find((model) => model.id === modelId)?.label ?? modelId;
}
