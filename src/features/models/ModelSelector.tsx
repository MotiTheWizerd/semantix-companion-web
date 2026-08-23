// The model picker, migrated from the studio's chat ModelSelector: one
// dropdown, two model families behind brand tabs — Semantix (the user's own
// OpenAI-compatible configured models) and Claude Code. Adapted to speak
// ModelPreference, the companion's model contract, so it drops into the
// companion form (and any future ModelPreference surface) as-is.

import { useState } from "react";
import { Dropdown } from "../../components/Dropdown/Dropdown";
import type { ConfiguredModel } from "./configuredModels/types";
import type { ModelPreference, UserPreferences } from "../preferences/types";
import { modelPreferenceValue } from "../preferences/types";
import { CLAUDE_MODELS, claudeModelLabel } from "./claudeCatalog";
import styles from "./ModelSelector.module.css";

/** The two model families the picker groups by. */
type ProviderFilter = "semantix" | "claude_code";

/** One row of the open menu — every row IS a preference, picked as-is. */
interface ModelRow {
  key: string;
  name: string;
  subtitle?: string;
  preference: ModelPreference;
}

interface ModelSelectorProps {
  value: ModelPreference;
  configuredModels: ConfiguredModel[];
  /** Needed only to name what "Default" currently resolves to. */
  userPreferences?: UserPreferences;
  /** The user default itself cannot inherit — it is what gets inherited. */
  allowInherit?: boolean;
  disabled?: boolean;
  onChange: (preference: ModelPreference) => void;
}

/** What the user default resolves to right now, named for the Default row. */
function inheritedLabel(
  configuredModels: ConfiguredModel[],
  userPreferences: UserPreferences | undefined,
): string {
  const inherited = userPreferences?.defaultModel;
  if (!inherited) return "Test stream";
  if (inherited.mode === "claude_code") {
    return `Claude · ${claudeModelLabel(inherited.modelId)}`;
  }
  if (inherited.mode !== "configured") return "Test stream";
  return (
    configuredModels.find((model) => model.id === inherited.modelId)
      ?.displayName ?? "Unavailable model"
  );
}

/** What the closed trigger says — never lies about what is actually set. */
function triggerLabel(
  value: ModelPreference,
  configuredModels: ConfiguredModel[],
  userPreferences: UserPreferences | undefined,
): string {
  switch (value.mode) {
    case "inherit":
      return `Default · ${inheritedLabel(configuredModels, userPreferences)}`;
    case "test":
      return "Test stream";
    case "claude_code":
      return claudeModelLabel(value.modelId);
    case "configured":
      return (
        configuredModels.find((model) => model.id === value.modelId)
          ?.displayName ?? "Unavailable model"
      );
  }
}

export function ModelSelector({
  value,
  configuredModels,
  userPreferences,
  allowInherit = false,
  disabled = false,
  onChange,
}: ModelSelectorProps) {
  // Which brand tab is active in the open menu. Null = follow the current
  // selection's family; a click pins the user's choice.
  const [filterOverride, setFilterOverride] = useState<ProviderFilter | null>(
    null,
  );

  const activeFilter: ProviderFilter =
    filterOverride ?? (value.mode === "claude_code" ? "claude_code" : "semantix");

  // Rows shared by both tabs: the Default row (when this surface may inherit)
  // stays reachable whichever family is open. The test stream is scaffolding,
  // not a model, so it is never OFFERED — it still renders when something is
  // already set to it, or the control would lie about what will answer.
  const pinnedRows: ModelRow[] = [
    ...(allowInherit
      ? [
          {
            key: "inherit",
            name: `Default · ${inheritedLabel(configuredModels, userPreferences)}`,
            preference: { mode: "inherit" } as ModelPreference,
          },
        ]
      : []),
    ...(value.mode === "test"
      ? [
          {
            key: "test",
            name: "Test stream",
            preference: { mode: "test" } as ModelPreference,
          },
        ]
      : []),
  ];

  const semantixRows: ModelRow[] = configuredModels.map((model) => ({
    key: `configured:${model.id}`,
    name: model.displayName,
    subtitle: model.providerId,
    preference: { mode: "configured", modelId: model.id },
  }));
  // A stored pick whose model is gone still gets a row — the picker stops
  // offering it fresh, it does not hide what is set.
  if (
    value.mode === "configured" &&
    !configuredModels.some((model) => model.id === value.modelId)
  ) {
    semantixRows.push({
      key: `configured:${value.modelId}`,
      name: "Unavailable model",
      preference: value,
    });
  }

  const claudeRows: ModelRow[] = CLAUDE_MODELS.map((model) => ({
    key: `claude_code:${model.id}`,
    name: model.label,
    subtitle: model.description,
    preference: { mode: "claude_code", modelId: model.id },
  }));
  if (
    value.mode === "claude_code" &&
    !CLAUDE_MODELS.some((model) => model.id === value.modelId)
  ) {
    claudeRows.push({
      key: `claude_code:${value.modelId}`,
      name: value.modelId,
      preference: value,
    });
  }

  const listItems = [
    ...pinnedRows,
    ...(activeFilter === "claude_code" ? claudeRows : semantixRows),
  ];
  const valueKey = modelPreferenceValue(value);
  const listValue = listItems.find((row) => row.key === valueKey) ?? null;
  const label = triggerLabel(value, configuredModels, userPreferences);

  return (
    <Dropdown
      items={listItems}
      value={listValue}
      onChange={(row) => onChange(row.preference)}
      getItemKey={(row) => row.key}
      renderItem={(row) => (
        <div className={styles.modelItem}>
          <span className={styles.modelName}>{row.name}</span>
          {row.subtitle && (
            <span className={styles.modelProvider}>{row.subtitle}</span>
          )}
        </div>
      )}
      renderTrigger={() => (
        <>
          <span className={styles.selectedName}>{label}</span>
          <svg
            className={styles.chevron}
            viewBox="0 0 24 24"
            width={11}
            height={11}
            fill="none"
            stroke="currentColor"
            strokeWidth={2}
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="m6 9 6 6 6-6" />
          </svg>
        </>
      )}
      placeholder="Select model"
      disabled={disabled}
      className={styles.dropdown}
      triggerClassName={styles.formTrigger}
      menuClassName={styles.menu}
      direction="down"
      searchable
      searchPlaceholder="Search models..."
      getSearchText={(row) => `${row.name} ${row.subtitle ?? ""}`}
      menuHeader={
        <div className={styles.brandRow}>
          <button
            type="button"
            className={`${styles.brandBtn} ${activeFilter === "semantix" ? styles.brandBtnActive : ""}`}
            onClick={(event) => {
              event.stopPropagation();
              setFilterOverride("semantix");
            }}
            title="Your models"
            aria-pressed={activeFilter === "semantix"}
          >
            <img
              className={`${styles.brandIcon} ${styles.brandIconSemantix}`}
              src="/semantix-icon.svg"
              alt="Semantix"
              draggable={false}
            />
          </button>
          <button
            type="button"
            className={`${styles.brandBtn} ${activeFilter === "claude_code" ? styles.brandBtnActive : ""}`}
            onClick={(event) => {
              event.stopPropagation();
              setFilterOverride("claude_code");
            }}
            title="Claude Code"
            aria-pressed={activeFilter === "claude_code"}
          >
            <img
              className={`${styles.brandIcon} ${styles.brandIconClaude}`}
              src="/ai-providers-icons/claude-code.svg"
              alt="Claude Code"
              draggable={false}
            />
          </button>
        </div>
      }
    />
  );
}
