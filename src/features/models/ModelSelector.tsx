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
  kind: ModelKind;
  preference: ModelPreference;
}

type ModelKind = "api" | "claude" | "test";

export interface ModelSelectorProps {
  value: ModelPreference;
  configuredModels: ConfiguredModel[];
  /** Needed only to name what "Default" currently resolves to. */
  userPreferences?: UserPreferences;
  /** The user default itself cannot inherit — it is what gets inherited. */
  allowInherit?: boolean;
  disabled?: boolean;
  id?: string;
  ariaLabel?: string;
  className?: string;
  direction?: "up" | "down";
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

function modelKind(
  preference: ModelPreference,
  userPreferences: UserPreferences | undefined,
): ModelKind {
  if (preference.mode === "inherit") {
    return userPreferences?.defaultModel.mode === "claude_code"
      ? "claude"
      : "api";
  }
  if (preference.mode === "claude_code") return "claude";
  if (preference.mode === "test") return "test";
  return "api";
}

function triggerDetail(
  value: ModelPreference,
  configuredModels: ConfiguredModel[],
): string {
  if (value.mode === "inherit") return "Inherited default";
  if (value.mode === "claude_code") return "Claude Code";
  if (value.mode === "test") return "Development fallback";
  return (
    configuredModels.find((model) => model.id === value.modelId)?.providerId ??
    "Configured API"
  );
}

function ModelMark({
  kind,
  className = "",
}: {
  kind: ModelKind;
  className?: string;
}) {
  if (kind === "test") {
    return (
      <span
        className={`${styles.modelMark} ${styles.modelMarkTest} ${className}`}
      >
        T
      </span>
    );
  }
  return (
    <span className={`${styles.modelMark} ${className}`}>
      <img
        src={
          kind === "claude"
            ? "/ai-providers-icons/claude-code.svg"
            : "/semantix-icon.svg"
        }
        alt=""
        draggable={false}
      />
    </span>
  );
}

export function ModelSelector({
  value,
  configuredModels,
  userPreferences,
  allowInherit = false,
  disabled = false,
  id,
  ariaLabel = "Model",
  className = "",
  direction = "down",
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
            subtitle: "Inherited default",
            kind: modelKind({ mode: "inherit" }, userPreferences),
            preference: { mode: "inherit" } as ModelPreference,
          },
        ]
      : []),
    ...(value.mode === "test"
      ? [
          {
            key: "test",
            name: "Test stream",
            subtitle: "Development fallback",
            kind: "test" as const,
            preference: { mode: "test" } as ModelPreference,
          },
        ]
      : []),
  ];

  const semantixRows: ModelRow[] = configuredModels.map((model) => ({
    key: `configured:${model.id}`,
    name: model.displayName,
    subtitle: model.providerId,
    kind: "api",
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
      subtitle: "Configured API",
      kind: "api",
      preference: value,
    });
  }

  const claudeRows: ModelRow[] = CLAUDE_MODELS.map((model) => ({
    key: `claude_code:${model.id}`,
    name: model.label,
    subtitle: model.description,
    kind: "claude",
    preference: { mode: "claude_code", modelId: model.id },
  }));
  if (
    value.mode === "claude_code" &&
    !CLAUDE_MODELS.some((model) => model.id === value.modelId)
  ) {
    claudeRows.push({
      key: `claude_code:${value.modelId}`,
      name: value.modelId,
      subtitle: "Claude Code",
      kind: "claude",
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
  const selectedKind = modelKind(value, userPreferences);

  return (
    <Dropdown
      items={listItems}
      value={listValue}
      onChange={(row) => onChange(row.preference)}
      getItemKey={(row) => row.key}
      renderItem={(row) => (
        <div className={styles.modelItem}>
          <ModelMark kind={row.kind} />
          <span className={styles.modelCopy}>
            <span className={styles.modelName}>{row.name}</span>
            {row.subtitle && (
              <span className={styles.modelProvider}>{row.subtitle}</span>
            )}
          </span>
        </div>
      )}
      renderTrigger={() => (
        <span className={styles.selectedModel}>
          <ModelMark kind={selectedKind} className={styles.selectedMark} />
          <span className={styles.selectedCopy}>
            <span className={styles.selectedName}>{label}</span>
            <span className={styles.selectedProvider}>
              {triggerDetail(value, configuredModels)}
            </span>
          </span>
        </span>
      )}
      placeholder="Select model"
      disabled={disabled}
      id={id}
      ariaLabel={ariaLabel}
      menuLabel="Available response models"
      className={`${styles.dropdown} ${className}`}
      triggerClassName={styles.formTrigger}
      menuClassName={styles.menu}
      direction={direction}
      searchable
      searchPlaceholder="Search models..."
      emptyMessage={
        activeFilter === "claude_code"
          ? "No Claude Code models available"
          : "No API models configured yet"
      }
      getSearchText={(row) => `${row.name} ${row.subtitle ?? ""}`}
      menuHeader={
        <div className={styles.menuHeader}>
          <div className={styles.menuHeading}>
            <span>Response model</span>
            <strong>Choose a model</strong>
          </div>
          <div
            className={styles.brandRow}
            role="group"
            aria-label="Model sources"
          >
            <button
              type="button"
              className={`${styles.brandBtn} ${activeFilter === "semantix" ? styles.brandBtnActive : ""}`}
              onClick={(event) => {
                event.stopPropagation();
                setFilterOverride("semantix");
              }}
              aria-pressed={activeFilter === "semantix"}
            >
              <ModelMark kind="api" className={styles.brandMark} />
              <span>API models</span>
              <small>{configuredModels.length}</small>
            </button>
            <button
              type="button"
              className={`${styles.brandBtn} ${activeFilter === "claude_code" ? styles.brandBtnActive : ""}`}
              onClick={(event) => {
                event.stopPropagation();
                setFilterOverride("claude_code");
              }}
              aria-pressed={activeFilter === "claude_code"}
            >
              <ModelMark kind="claude" className={styles.brandMark} />
              <span>Claude Code</span>
              <small>{CLAUDE_MODELS.length}</small>
            </button>
          </div>
        </div>
      }
    />
  );
}
