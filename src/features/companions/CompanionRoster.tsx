import { useEffect, useState, type FormEvent } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import { ConfirmDeleteButton } from "../../components/ConfirmDeleteButton";
import { EditButton } from "../../components/EditButton";
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

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Companion could not update your companions.";
}

/** One glyph for the row: the name's first letter, or a mark for the unnamed. */
function companionInitial(companion: Companion): string {
  return companion.name?.trim().charAt(0).toUpperCase() || "◦";
}

export function CompanionRoster() {
  const [companions, setCompanions] = useState<Companion[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isFormOpen, setIsFormOpen] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [name, setName] = useState("");
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

        const roster = await listCompanions();
        if (!cancelled) setCompanions(roster);
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
    setError(null);
    setEditingId(null);
    setIsFormOpen(false);
  };

  const openCreateForm = () => {
    setName("");
    setError(null);
    setEditingId(null);
    setPendingDeleteId(null);
    setIsFormOpen(true);
  };

  const openEditForm = (companion: Companion) => {
    setName(companion.name ?? "");
    setError(null);
    setEditingId(companion.id);
    setPendingDeleteId(null);
    setIsFormOpen(true);
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();

    // A blank name is legal — Rust normalises it back to unnamed.
    const submitted = name.trim() ? name.trim() : null;
    setError(null);
    setIsSaving(true);
    try {
      const companion = isEditing
        ? await updateCompanion({ companionId: editingId, name: submitted })
        : await createCompanion({ name: submitted });
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
          <p>Each companion keeps its own private memory. Naming one is optional.</p>
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
                  <span>{companion.isBuiltIn ? "Built-in companion" : "Companion"}</span>
                </div>
                <code>Private memory</code>
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
