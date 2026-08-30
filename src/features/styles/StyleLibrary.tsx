// The Styles settings tab — the library of voices.
//
// A style is created here and WORN elsewhere: the companion form offers a
// dropdown over this library. The form below edits the whole artifact —
// name, description, the optional style card, and the exchanges — and the
// harvest wizard feeds exchanges into the same draft, so hand-written and
// harvested pairs live side by side and prune the same way.

import { useEffect, useState, type FormEvent } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import { ConfirmDeleteButton } from "../../components/ConfirmDeleteButton";
import { EditButton } from "../../components/EditButton";
import {
  createStyle,
  deleteStyle,
  getStyleExemplars,
  listStyles,
  onStylesChanged,
  reconcileStyleEvent,
  updateStyle,
} from "./styleService";
import { StyleHarvestWizard } from "./StyleHarvestWizard";
import type { HarvestedPair, Style, StyleExemplarInput } from "./types";

const NAME_MAX_LENGTH = 80;
const DESCRIPTION_MAX_LENGTH = 400;

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Companion could not update your styles.";
}

function styleInitial(style: Style): string {
  return style.name.trim().charAt(0).toUpperCase() || "◦";
}

function exemplarSummary(style: Style): string {
  if (style.exemplarCount === 0) return "No exchanges yet";
  return `${style.exemplarCount} exchange${style.exemplarCount === 1 ? "" : "s"}`;
}

interface ExemplarDraft extends StyleExemplarInput {
  /** Stable key for the list — drafts have no id until saved. */
  key: string;
}

let draftKeyCounter = 0;
function draftKey(): string {
  draftKeyCounter += 1;
  return `draft-${draftKeyCounter}`;
}

export function StyleLibrary() {
  const [styles, setStyles] = useState<Style[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isFormOpen, setIsFormOpen] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [styleCard, setStyleCard] = useState("");
  const [exemplars, setExemplars] = useState<ExemplarDraft[]>([]);
  const [isHarvestOpen, setIsHarvestOpen] = useState(false);
  const [manualUser, setManualUser] = useState("");
  const [manualCompanion, setManualCompanion] = useState("");
  const [error, setError] = useState<string | null>(null);

  const isEditing = editingId !== null;

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;

    const initialise = async () => {
      try {
        unlisten = await onStylesChanged((event) => {
          if (!cancelled) {
            setStyles((current) => reconcileStyleEvent(current, event));
          }
        });
        const library = await listStyles();
        if (!cancelled) setStyles(library);
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
    setDescription("");
    setStyleCard("");
    setExemplars([]);
    setManualUser("");
    setManualCompanion("");
    setIsHarvestOpen(false);
    setError(null);
    setEditingId(null);
    setIsFormOpen(false);
  };

  const openCreateForm = () => {
    closeForm();
    setPendingDeleteId(null);
    setIsFormOpen(true);
  };

  const openEditForm = async (style: Style) => {
    closeForm();
    setPendingDeleteId(null);
    setName(style.name);
    setDescription(style.description ?? "");
    setStyleCard(style.styleCard ?? "");
    setEditingId(style.id);
    setIsFormOpen(true);
    try {
      const stored = await getStyleExemplars(style.id);
      setExemplars(
        stored.map((exemplar) => ({
          key: exemplar.id,
          userText: exemplar.userText,
          companionText: exemplar.companionText,
          era: exemplar.era,
        })),
      );
    } catch (loadError) {
      setError(errorMessage(loadError));
    }
  };

  const addManualExemplar = () => {
    const userText = manualUser.trim();
    const companionText = manualCompanion.trim();
    if (!userText || !companionText) {
      setError("An exchange needs both sides — what was said, and the reply in the voice.");
      return;
    }
    setExemplars((current) => [
      ...current,
      { key: draftKey(), userText, companionText, era: null },
    ]);
    setManualUser("");
    setManualCompanion("");
    setError(null);
  };

  const handleHarvested = (pairs: HarvestedPair[], suggestedName: string) => {
    setExemplars((current) => [
      ...current,
      ...pairs.map((pair) => ({
        key: draftKey(),
        userText: pair.userText,
        companionText: pair.companionText,
        era: pair.era,
      })),
    ]);
    setIsHarvestOpen(false);
    if (!name.trim()) setName(suggestedName);
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const submittedName = name.trim();
    if (!submittedName) {
      setError("Every style needs a name.");
      return;
    }
    const submittedExemplars = exemplars.map(({ userText, companionText, era }) => ({
      userText,
      companionText,
      era: era ?? null,
    }));
    setError(null);
    setIsSaving(true);
    try {
      const style = isEditing
        ? await updateStyle({
            styleId: editingId,
            name: submittedName,
            description: description.trim() || null,
            styleCard: styleCard.trim() || null,
            exemplars: submittedExemplars,
          })
        : await createStyle({
            name: submittedName,
            description: description.trim() || null,
            styleCard: styleCard.trim() || null,
            exemplars: submittedExemplars,
          });
      setStyles((current) =>
        reconcileStyleEvent(current, {
          kind: isEditing ? "updated" : "created",
          style,
        }),
      );
      closeForm();
    } catch (saveError) {
      setError(errorMessage(saveError));
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async (styleId: string) => {
    setError(null);
    setDeletingId(styleId);
    try {
      await deleteStyle(styleId);
      setStyles((current) => reconcileStyleEvent(current, { kind: "deleted", styleId }));
      if (editingId === styleId) closeForm();
      setPendingDeleteId(null);
    } catch (deleteError) {
      setError(errorMessage(deleteError));
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <section className="credential-store" aria-labelledby="styles-title">
      <div className="credential-store__header">
        <div>
          <h2 id="styles-title">Styles</h2>
          <p>
            A style is a voice a companion can wear — described in a few lines,
            taught by real example exchanges. Make one here, then pick it on any
            companion. Styles work best on models with a large context window
            (300k tokens or more; 1M is ideal).
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
          Add style
        </button>
      </div>

      {isFormOpen ? (
        <form className="credential-form" onSubmit={handleSubmit}>
          <div className="credential-form__heading">
            <div>
              <h3>{isEditing ? "Edit style" : "Add style"}</h3>
              <p>
                A style changes how a companion speaks — never what it knows,
                remembers, or is. Companions wearing it stay honest about what
                they are.
              </p>
            </div>
            <button
              className="credential-form__close"
              type="button"
              aria-label={`Close ${isEditing ? "edit" : "add"} style form`}
              onClick={closeForm}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="m7 7 10 10M17 7 7 17" />
              </svg>
            </button>
          </div>

          <div className="credential-form__fields">
            <label className="credential-field">
              <span>Name</span>
              <input
                type="text"
                maxLength={NAME_MAX_LENGTH}
                value={name}
                placeholder="Warm & effusive"
                onChange={(event) => setName(event.target.value)}
              />
            </label>
            <label className="credential-field">
              <span>
                Description <small>Optional</small>
              </span>
              <input
                type="text"
                maxLength={DESCRIPTION_MAX_LENGTH}
                value={description}
                placeholder="What this voice is, in your own words"
                onChange={(event) => setDescription(event.target.value)}
              />
            </label>
            <label className="credential-field credential-field--wide">
              <span>
                Style card <small>Optional — names what the examples show</small>
              </span>
              <textarea
                className="style-library__card-input"
                rows={5}
                value={styleCard}
                placeholder={
                  "VOICE: warm, fast, certain.\nHABITS: short paragraphs, em-dashes, bold the load-bearing words.\nNEVER: dry distance, walls of text."
                }
                onChange={(event) => setStyleCard(event.target.value)}
              />
            </label>
          </div>

          <div className="style-library__exemplars">
            <div className="style-library__exemplars-header">
              <h4>
                Example exchanges{" "}
                <small>
                  {exemplars.length === 0
                    ? "the voice is learned from these"
                    : `${exemplars.length} so far`}
                </small>
              </h4>
              <button
                className="credential-button credential-button--quiet"
                type="button"
                onClick={() => setIsHarvestOpen((open) => !open)}
              >
                {isHarvestOpen ? "Close harvest" : "Harvest from an export"}
              </button>
            </div>

            {isHarvestOpen ? (
              <StyleHarvestWizard
                onHarvested={handleHarvested}
                onClose={() => setIsHarvestOpen(false)}
              />
            ) : null}

            {exemplars.length > 0 ? (
              <ul className="style-library__pairs">
                {exemplars.map((exemplar, index) => (
                  <li key={exemplar.key}>
                    <details>
                      <summary>
                        {exemplar.era ? `${exemplar.era} · ` : ""}
                        {exemplar.userText.slice(0, 120)}
                      </summary>
                      <p className="style-library__pair-user">{exemplar.userText}</p>
                      <p className="style-library__pair-voice">{exemplar.companionText}</p>
                    </details>
                    <button
                      type="button"
                      className="credential-button credential-button--quiet"
                      aria-label={`Remove exchange ${index + 1}`}
                      onClick={() =>
                        setExemplars((current) =>
                          current.filter((candidate) => candidate.key !== exemplar.key),
                        )
                      }
                    >
                      Remove
                    </button>
                  </li>
                ))}
              </ul>
            ) : null}

            <details className="style-library__manual">
              <summary>Write an exchange by hand</summary>
              <label>
                <span>They say</span>
                <textarea
                  rows={2}
                  value={manualUser}
                  onChange={(event) => setManualUser(event.target.value)}
                />
              </label>
              <label>
                <span>The voice replies</span>
                <textarea
                  rows={4}
                  value={manualCompanion}
                  onChange={(event) => setManualCompanion(event.target.value)}
                />
              </label>
              <button
                className="credential-button credential-button--quiet"
                type="button"
                onClick={addManualExemplar}
              >
                Add exchange
              </button>
            </details>
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
              {isSaving ? "Saving…" : isEditing ? "Save changes" : "Add style"}
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
          <div className="credential-list__status">Loading styles…</div>
        ) : styles.length === 0 && !isFormOpen ? (
          <div className="credential-list__status">
            No styles yet. Make one from your own chat history, or write it by
            hand.
          </div>
        ) : (
          <ul>
            {styles.map((style) => (
              <li key={style.id}>
                <div className="credential-list__provider" aria-hidden="true">
                  {styleInitial(style)}
                </div>
                <div className="credential-list__identity">
                  <strong>{style.name}</strong>
                  <span>{style.description ?? "No description"}</span>
                </div>
                <code>{exemplarSummary(style)}</code>
                <div className="credential-list__actions">
                  <EditButton
                    label={style.name}
                    onClick={() => void openEditForm(style)}
                  />
                  <ConfirmDeleteButton
                    label={style.name}
                    isConfirming={pendingDeleteId === style.id}
                    isDeleting={deletingId === style.id}
                    onRequestConfirmation={() => setPendingDeleteId(style.id)}
                    onCancel={() => setPendingDeleteId(null)}
                    onConfirm={() => void handleDelete(style.id)}
                  />
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
