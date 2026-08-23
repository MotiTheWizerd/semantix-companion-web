import { useEffect, useState, type FormEvent } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { open as openFolderDialog } from "@tauri-apps/plugin-dialog";

import { ConfirmDeleteButton } from "../../components/ConfirmDeleteButton";
import { EditButton } from "../../components/EditButton";
import { listConfiguredModels } from "../models/configuredModels/modelService";
import type { ConfiguredModel } from "../models/configuredModels/types";
import { ModelPreferenceSelect } from "../preferences/ModelPreferenceSelect";
import { getUserPreferences } from "../preferences/preferenceService";
import type { ModelPreference, UserPreferences } from "../preferences/types";
import {
  createCompanion,
  deleteCompanion,
  listCompanions,
  onCompanionsChanged,
  reconcileCompanionEvent,
  updateCompanion,
} from "./companionService";
import { companionLabel, type Companion } from "./types";

const NAME_MAX_LENGTH = 80;

/** A new companion follows the user's default model until told otherwise. */
const DEFAULT_MODEL_PREFERENCE: ModelPreference = { mode: "inherit" };

const EMPTY_USER_PREFERENCES: UserPreferences = {
  defaultModel: { mode: "test" },
  updatedAt: 0,
};

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Companion could not update your companions.";
}

/** One glyph for the row: the name's first letter, or a mark for the unnamed. */
function companionInitial(companion: Companion): string {
  return companion.name?.trim().charAt(0).toUpperCase() || "◦";
}

/** The folder's own name — the full path lives in the edit form. */
function folderBasename(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** The voice named for the row, resolved the way a send would resolve it. */
function voiceLabel(
  companion: Companion,
  configuredModels: ConfiguredModel[],
  userPreferences: UserPreferences,
): string {
  const preference =
    companion.modelPreference.mode === "inherit"
      ? userPreferences.defaultModel
      : companion.modelPreference;
  if (preference.mode !== "configured") return "Test stream";
  return (
    configuredModels.find((model) => model.id === preference.modelId)?.displayName ??
    "Unavailable model"
  );
}

export function CompanionRoster() {
  const [companions, setCompanions] = useState<Companion[]>([]);
  const [configuredModels, setConfiguredModels] = useState<ConfiguredModel[]>([]);
  const [userPreferences, setUserPreferences] = useState<UserPreferences>(
    EMPTY_USER_PREFERENCES,
  );
  const [isLoading, setIsLoading] = useState(true);
  const [isFormOpen, setIsFormOpen] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [modelPreference, setModelPreference] = useState<ModelPreference>(
    DEFAULT_MODEL_PREFERENCE,
  );
  const [workspaceDir, setWorkspaceDir] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const isEditing = editingId !== null;

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;

    const initialise = async () => {
      try {
        unlisten = await onCompanionsChanged((event) => {
          if (!cancelled) {
            setCompanions((current) => reconcileCompanionEvent(current, event));
          }
        });

        const [roster, models, preferences] = await Promise.all([
          listCompanions(),
          listConfiguredModels(),
          getUserPreferences(),
        ]);
        if (cancelled) return;
        setCompanions(roster);
        setConfiguredModels(models);
        setUserPreferences(preferences);
      } catch (initialisationError) {
        if (!cancelled) setError(errorMessage(initialisationError));
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };

    void initialise();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const closeForm = () => {
    setName("");
    setModelPreference(DEFAULT_MODEL_PREFERENCE);
    setWorkspaceDir(null);
    setError(null);
    setEditingId(null);
    setIsFormOpen(false);
  };

  const openCreateForm = () => {
    setName("");
    setModelPreference(DEFAULT_MODEL_PREFERENCE);
    setWorkspaceDir(null);
    setError(null);
    setEditingId(null);
    setPendingDeleteId(null);
    setIsFormOpen(true);
  };

  const openEditForm = (companion: Companion) => {
    setName(companion.name ?? "");
    setModelPreference(companion.modelPreference);
    setWorkspaceDir(companion.workspaceDir);
    setError(null);
    setEditingId(companion.id);
    setPendingDeleteId(null);
    setIsFormOpen(true);
  };

  const pickWorkspaceFolder = async () => {
    try {
      const picked = await openFolderDialog({
        directory: true,
        title: "Choose a workspace folder",
        defaultPath: workspaceDir ?? undefined,
      });
      if (typeof picked === "string" && picked) setWorkspaceDir(picked);
    } catch (pickError) {
      setError(errorMessage(pickError));
    }
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    // A blank name is legal — Rust normalises it back to unnamed.
    const submitted = name.trim() ? name.trim() : null;
    setError(null);
    setIsSaving(true);
    try {
      const companion = isEditing
        ? await updateCompanion({
            companionId: editingId,
            name: submitted,
            modelPreference,
            workspaceDir,
          })
        : await createCompanion({ name: submitted, modelPreference, workspaceDir });
      setCompanions((current) =>
        reconcileCompanionEvent(current, {
          kind: isEditing ? "updated" : "created",
          companion,
        }),
      );
      closeForm();
    } catch (saveError) {
      setError(errorMessage(saveError));
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async (companionId: string) => {
    setError(null);
    setDeletingId(companionId);
    try {
      await deleteCompanion(companionId);
      setCompanions((current) =>
        reconcileCompanionEvent(current, { kind: "deleted", companionId }),
      );
      if (editingId === companionId) closeForm();
      setPendingDeleteId(null);
    } catch (deleteError) {
      setError(errorMessage(deleteError));
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <section className="credential-store" aria-labelledby="companions-title">
      <div className="credential-store__header">
        <div>
          <h2 id="companions-title">Companions</h2>
          <p>
            Each companion keeps its own private memory and answers with its own
            model. Naming one is optional.
          </p>
        </div>
        <button
          className="credential-store__add-button"
          type="button"
          aria-expanded={isFormOpen}
          onClick={() => (isFormOpen ? closeForm() : openCreateForm())}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 5v14M5 12h14" />
          </svg>
          Add companion
        </button>
      </div>

      {isFormOpen ? (
        <form className="credential-form" onSubmit={handleSubmit}>
          <div className="credential-form__heading">
            <div>
              <h3>{isEditing ? "Edit companion" : "Add companion"}</h3>
              <p>
                {isEditing
                  ? "Renaming a companion leaves its memory exactly where it is."
                  : "A new companion starts with an empty memory of its own."}
              </p>
            </div>
            <button
              className="credential-form__close"
              type="button"
              aria-label={`Close ${isEditing ? "edit" : "add"} companion form`}
              onClick={closeForm}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="m7 7 10 10M17 7 7 17" />
              </svg>
            </button>
          </div>

          <div className="credential-form__fields">
            <label className="credential-field credential-field--wide">
              <span>
                Name <small>Optional</small>
              </span>
              <input
                type="text"
                maxLength={NAME_MAX_LENGTH}
                value={name}
                placeholder="Leave blank to keep this companion unnamed"
                onChange={(event) => setName(event.target.value)}
              />
            </label>

            <label className="credential-field credential-field--wide">
              <span>Model</span>
              <ModelPreferenceSelect
                value={modelPreference}
                configuredModels={configuredModels}
                userPreferences={userPreferences}
                allowInherit
                onChange={setModelPreference}
              />
            </label>

            <div className="credential-field credential-field--wide">
              <span>
                Workspace folder <small>Optional</small>
              </span>
              <div className="credential-field__workspace">
                <button
                  type="button"
                  className="credential-button credential-button--quiet"
                  onClick={() => void pickWorkspaceFolder()}
                >
                  {workspaceDir ? "Change folder…" : "Choose folder…"}
                </button>
                {workspaceDir ? (
                  <>
                    <code title={workspaceDir}>{workspaceDir}</code>
                    <button
                      type="button"
                      className="credential-button credential-button--quiet"
                      aria-label="Remove the workspace folder"
                      onClick={() => setWorkspaceDir(null)}
                    >
                      Remove
                    </button>
                  </>
                ) : (
                  <small>
                    With a workspace, this companion can list, read, write, edit
                    and delete files — inside that one folder only.
                  </small>
                )}
              </div>
            </div>
          </div>

          {error ? (
            <p className="credential-store__error" role="alert">
              {error}
            </p>
          ) : null}

          <div className="credential-form__actions">
            <button
              type="button"
              className="credential-button credential-button--quiet"
              onClick={closeForm}
            >
              Cancel
            </button>
            <button
              type="submit"
              className="credential-button credential-button--primary"
              disabled={isSaving}
            >
              {isSaving ? "Saving…" : isEditing ? "Save changes" : "Add companion"}
            </button>
          </div>
        </form>
      ) : null}

      {!isFormOpen && error ? (
        <p className="credential-store__error" role="alert">
          {error}
        </p>
      ) : null}

      <div className="credential-list" aria-live="polite" aria-busy={isLoading}>
        {isLoading ? (
          <div className="credential-list__status">Loading companions…</div>
        ) : (
          <ul>
            {companions.map((companion) => (
              <li key={companion.id}>
                <div className="credential-list__provider" aria-hidden="true">
                  {companionInitial(companion)}
                </div>
                <div className="credential-list__identity">
                  <strong>{companionLabel(companion)}</strong>
                  <span>
                    {voiceLabel(companion, configuredModels, userPreferences)}
                    {companion.isBuiltIn ? " · Built-in" : ""}
                  </span>
                </div>
                <code>
                  {companion.workspaceDir
                    ? `Private memory · 📁 ${folderBasename(companion.workspaceDir)}`
                    : "Private memory"}
                </code>
                <div className="credential-list__actions">
                  <EditButton
                    label={companionLabel(companion)}
                    onClick={() => openEditForm(companion)}
                  />
                  {companion.isBuiltIn ? null : (
                    <ConfirmDeleteButton
                      label={companionLabel(companion)}
                      isConfirming={pendingDeleteId === companion.id}
                      isDeleting={deletingId === companion.id}
                      onRequestConfirmation={() => setPendingDeleteId(companion.id)}
                      onCancel={() => setPendingDeleteId(null)}
                      onConfirm={() => void handleDelete(companion.id)}
                    />
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
