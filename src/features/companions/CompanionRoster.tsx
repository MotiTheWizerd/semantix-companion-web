import { useEffect, useState, type FormEvent } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import { ConfirmDeleteButton } from "../../components/ConfirmDeleteButton";
import { EditButton } from "../../components/EditButton";
import { ImportWizard } from "../import/ImportWizard";
import { claudeModelLabel } from "../models/claudeCatalog";
import { listConfiguredModels } from "../models/configuredModels/modelService";
import type { ConfiguredModel } from "../models/configuredModels/types";
import { ModelSelector } from "../models/ModelSelector";
import { getUserPreferences } from "../preferences/preferenceService";
import type { ModelPreference, UserPreferences } from "../preferences/types";
import { listStyles, onStylesChanged, reconcileStyleEvent } from "../styles/styleService";
import type { Style } from "../styles/types";
import {
  clearCompanionAvatar,
  createCompanion,
  deleteCompanion,
  listCompanions,
  onCompanionsChanged,
  reconcileCompanionEvent,
  setCompanionAvatar,
  updateCompanion,
} from "./companionService";
import { companionLabel, type Companion } from "./types";
import {
  WorkspaceFolderEditor,
  workspaceDraftError,
  type WorkspaceFolderDraft,
} from "./WorkspaceFolderEditor";

const NAME_MAX_LENGTH = 80;

/** A new companion follows the user's default model until told otherwise. */
const DEFAULT_MODEL_PREFERENCE: ModelPreference = { mode: "inherit" };

const EMPTY_USER_PREFERENCES: UserPreferences = {
  defaultModel: { mode: "test" },
  displayName: null,
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

/** The last segment of a picked path — all the form needs to confirm the
 *  choice, and the only part of an absolute path worth showing a user. */
function fileBaseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

function workspaceSummary(companion: Companion): string {
  if (companion.workspaces.length === 0) return "Private memory";
  const labels = companion.workspaces.map((workspace) => workspace.label);
  const visible = labels.slice(0, 2).join(" · ");
  const remaining = labels.length - 2;
  return `Private memory · 📁 ${visible}${remaining > 0 ? ` · +${remaining}` : ""}`;
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
  if (preference.mode === "claude_code") {
    return `Claude · ${claudeModelLabel(preference.modelId)}`;
  }
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
  const [workspaces, setWorkspaces] = useState<WorkspaceFolderDraft[]>([]);
  const [availableStyles, setAvailableStyles] = useState<Style[]>([]);
  const [styleId, setStyleId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [importingId, setImportingId] = useState<string | null>(null);
  const [avatarBusyId, setAvatarBusyId] = useState<string | null>(null);
  /** A picture chosen while CREATING a companion, held until there is an id to
   *  file it under — avatars are stored by companion id, so one cannot be
   *  written before the companion exists. Applied right after the create. */
  const [pendingAvatarPath, setPendingAvatarPath] = useState<string | null>(null);

  const isEditing = editingId !== null;
  /** The live record behind the open edit form. Read from the roster rather
   *  than copied into form state, so a picture set from the form (which
   *  applies at once) shows up in it immediately. */
  const editingCompanion =
    companions.find((companion) => companion.id === editingId) ?? null;
  const workspaceValidationError = workspaceDraftError(workspaces);
  const importingCompanion =
    companions.find((companion) => companion.id === importingId) ?? null;

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;

    let unlistenStyles: UnlistenFn | undefined;

    const initialise = async () => {
      try {
        unlisten = await onCompanionsChanged((event) => {
          if (!cancelled) {
            setCompanions((current) => reconcileCompanionEvent(current, event));
          }
        });
        unlistenStyles = await onStylesChanged((event) => {
          if (!cancelled) {
            setAvailableStyles((current) => reconcileStyleEvent(current, event));
          }
        });

        const [roster, models, preferences, styleLibrary] = await Promise.all([
          listCompanions(),
          listConfiguredModels(),
          getUserPreferences(),
          listStyles(),
        ]);
        if (cancelled) return;
        setCompanions(roster);
        setConfiguredModels(models);
        setUserPreferences(preferences);
        setAvailableStyles(styleLibrary);
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
      unlistenStyles?.();
    };
  }, []);

  const closeForm = () => {
    setName("");
    setModelPreference(DEFAULT_MODEL_PREFERENCE);
    setWorkspaces([]);
    setStyleId(null);
    setError(null);
    setEditingId(null);
    setPendingAvatarPath(null);
    setIsFormOpen(false);
  };

  const openCreateForm = () => {
    setName("");
    setModelPreference(DEFAULT_MODEL_PREFERENCE);
    setWorkspaces([]);
    setStyleId(null);
    setError(null);
    setEditingId(null);
    setPendingDeleteId(null);
    setImportingId(null);
    setPendingAvatarPath(null);
    setIsFormOpen(true);
  };

  const openImportWizard = (companion: Companion) => {
    closeForm();
    setPendingDeleteId(null);
    setImportingId(companion.id);
  };

  const openEditForm = (companion: Companion) => {
    setImportingId(null);
    setName(companion.name ?? "");
    setModelPreference(companion.modelPreference);
    setStyleId(companion.styleId);
    setWorkspaces(
      companion.workspaces.map((workspace) => ({
        key: workspace.id,
        id: workspace.id,
        label: workspace.label,
        directory: workspace.directory,
      })),
    );
    setError(null);
    setEditingId(companion.id);
    setPendingDeleteId(null);
    setIsFormOpen(true);
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    // A blank name is legal — Rust normalises it back to unnamed.
    const submitted = name.trim() ? name.trim() : null;
    const workspaceError = workspaceDraftError(workspaces);
    if (workspaceError) {
      setError(workspaceError);
      return;
    }
    const submittedWorkspaces = workspaces.map(({ id, label, directory }) => ({
      id,
      label: label.trim(),
      directory,
    }));
    setError(null);
    setIsSaving(true);
    try {
      let companion = isEditing
        ? await updateCompanion({
            companionId: editingId,
            name: submitted,
            modelPreference,
            workspaces: submittedWorkspaces,
            styleId,
          })
        : await createCompanion({
            name: submitted,
            modelPreference,
            workspaces: submittedWorkspaces,
            styleId,
          });
      // The companion now has an id, so a picture chosen during the create can
      // finally be filed. A failure here must not lose the companion that was
      // just made, so it surfaces as an error over a saved record.
      if (!isEditing && pendingAvatarPath) {
        companion = await setCompanionAvatar(companion.id, pendingAvatarPath);
      }
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

  /** The native image chooser. `null` means the user cancelled. */
  const pickImageFile = async (subject: string): Promise<string | null> => {
    const selection = await openFileDialog({
      multiple: false,
      directory: false,
      title: `Choose a picture for ${subject}`,
      filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "webp"] }],
    });
    return typeof selection === "string" ? selection : null;
  };

  /** Set the picture of a companion that already exists.
   *
   *  Applied at once rather than on Save: a face comes from a native dialog,
   *  and holding it until Submit would let a form opened before the choice
   *  quietly revert it. The roster updates from the returned companion, and
   *  `companions://changed` carries it to every other surface.
   *
   *  Reached from BOTH the row's picture button and the edit form's Picture
   *  field — one path, two doors. */
  const handleChooseAvatar = async (companion: Companion) => {
    const selection = await pickImageFile(companionLabel(companion));
    if (!selection) return;

    setError(null);
    setAvatarBusyId(companion.id);
    try {
      const updated = await setCompanionAvatar(companion.id, selection);
      setCompanions((current) =>
        reconcileCompanionEvent(current, { kind: "updated", companion: updated }),
      );
    } catch (avatarError) {
      setError(errorMessage(avatarError));
    } finally {
      setAvatarBusyId(null);
    }
  };

  /** Choosing a picture for a companion that does not exist yet: remember the
   *  path, and let the create carry it once there is an id to file it under. */
  const handleChoosePendingAvatar = async () => {
    const selection = await pickImageFile("the new companion");
    if (selection) setPendingAvatarPath(selection);
  };

  const handleClearAvatar = async (companion: Companion) => {
    setError(null);
    setAvatarBusyId(companion.id);
    try {
      const updated = await clearCompanionAvatar(companion.id);
      setCompanions((current) =>
        reconcileCompanionEvent(current, { kind: "updated", companion: updated }),
      );
    } catch (avatarError) {
      setError(errorMessage(avatarError));
    } finally {
      setAvatarBusyId(null);
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

            <div className="credential-field credential-field--wide">
              <span>
                Picture <small>Optional</small>
              </span>
              <div className="avatar-field">
                <span
                  className={`avatar-field__preview${
                    editingCompanion?.avatarUrl ? " avatar-field__preview--has-image" : ""
                  }`}
                  aria-hidden="true"
                >
                  {editingCompanion?.avatarUrl ? (
                    <img src={editingCompanion.avatarUrl} alt="" draggable={false} />
                  ) : (
                    <img src="/logo-mark.png" alt="" draggable={false} />
                  )}
                </span>
                <div className="avatar-field__controls">
                  <button
                    className="avatar-field__button"
                    type="button"
                    disabled={editingId !== null && avatarBusyId === editingId}
                    onClick={() =>
                      void (editingCompanion
                        ? handleChooseAvatar(editingCompanion)
                        : handleChoosePendingAvatar())
                    }
                  >
                    {editingCompanion?.avatarUrl || pendingAvatarPath
                      ? "Change image…"
                      : "Choose image…"}
                  </button>
                  {editingCompanion?.avatarUrl ? (
                    <button
                      className="avatar-field__button avatar-field__button--quiet"
                      type="button"
                      disabled={editingId !== null && avatarBusyId === editingId}
                      onClick={() => void handleClearAvatar(editingCompanion)}
                    >
                      Remove
                    </button>
                  ) : null}
                  <small className="avatar-field__hint">
                    {pendingAvatarPath
                      ? `${fileBaseName(pendingAvatarPath)} — added when you save`
                      : "PNG, JPEG, GIF or WebP, up to 4 MB. Without one, the companion wears the mark."}
                  </small>
                </div>
              </div>
            </div>

            <div className="credential-field credential-field--wide">
              <span>Model</span>
              <ModelSelector
                value={modelPreference}
                configuredModels={configuredModels}
                userPreferences={userPreferences}
                allowInherit
                ariaLabel="Companion model"
                onChange={setModelPreference}
              />
            </div>

            <label className="credential-field credential-field--wide">
              <span>
                Style <small>Optional — a voice from your style library</small>
              </span>
              <select
                aria-label="Companion style"
                value={styleId ?? ""}
                onChange={(event) => setStyleId(event.target.value || null)}
              >
                <option value="">No style — speaks plainly</option>
                {availableStyles.map((style) => (
                  <option key={style.id} value={style.id}>
                    {style.name}
                    {style.exemplarCount > 0
                      ? ` (${style.exemplarCount} exchanges)`
                      : ""}
                  </option>
                ))}
              </select>
            </label>

            <div className="credential-field credential-field--wide">
              <WorkspaceFolderEditor
                value={workspaces}
                disabled={isSaving}
                onChange={(next) => {
                  setWorkspaces(next);
                  setError(null);
                }}
                onError={(pickerError) => setError(errorMessage(pickerError))}
              />
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
              disabled={isSaving || workspaceValidationError !== null}
            >
              {isSaving ? "Saving…" : isEditing ? "Save changes" : "Add companion"}
            </button>
          </div>
        </form>
      ) : null}

      {importingCompanion ? (
        <ImportWizard
          companion={importingCompanion}
          onClose={() => setImportingId(null)}
        />
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
                <div className="companion-avatar">
                  <button
                    className={`credential-list__provider companion-avatar__pick${
                      companion.avatarUrl ? " companion-avatar__pick--has-image" : ""
                    }`}
                    type="button"
                    disabled={avatarBusyId === companion.id}
                    title={companion.avatarUrl ? "Change picture" : "Add a picture"}
                    aria-label={`${
                      companion.avatarUrl ? "Change" : "Add"
                    } a picture for ${companionLabel(companion)}`}
                    onClick={() => void handleChooseAvatar(companion)}
                  >
                    {companion.avatarUrl ? (
                      <img src={companion.avatarUrl} alt="" draggable={false} />
                    ) : (
                      <span aria-hidden="true">{companionInitial(companion)}</span>
                    )}
                  </button>
                  {companion.avatarUrl ? (
                    <button
                      className="companion-avatar__clear"
                      type="button"
                      disabled={avatarBusyId === companion.id}
                      title="Remove picture"
                      aria-label={`Remove the picture for ${companionLabel(companion)}`}
                      onClick={() => void handleClearAvatar(companion)}
                    >
                      <svg viewBox="0 0 24 24" aria-hidden="true">
                        <path d="M7 7l10 10M17 7L7 17" />
                      </svg>
                    </button>
                  ) : null}
                </div>
                <div className="credential-list__identity">
                  <strong>{companionLabel(companion)}</strong>
                  <span>
                    {voiceLabel(companion, configuredModels, userPreferences)}
                    {companion.styleId
                      ? ` · ${
                          availableStyles.find(
                            (style) => style.id === companion.styleId,
                          )?.name ?? "Style"
                        }`
                      : ""}
                    {companion.isBuiltIn ? " · Built-in" : ""}
                  </span>
                </div>
                <code title={companion.workspaces.map(({ label }) => label).join(", ")}>
                  {workspaceSummary(companion)}
                </code>
                <div className="credential-list__actions">
                  <button
                    className="credential-list__edit"
                    type="button"
                    title="Bring your history — import old Claude or ChatGPT chats"
                    aria-label={`Import history into ${companionLabel(companion)}`}
                    onClick={() => openImportWizard(companion)}
                  >
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path d="M12 4v10m0 0 4-4m-4 4-4-4" />
                      <path d="M5 18h14" />
                    </svg>
                  </button>
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
