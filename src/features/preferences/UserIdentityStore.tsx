import { useEffect, useState, type FormEvent } from "react";
import { useShallow } from "zustand/react/shallow";

import { useCompanionStore } from "../workspace/companionStore";

/** Your name, and nothing else yet. It lives on the same user_preferences row
 *  as the default model, but it is not a model setting — a person's name is the
 *  first thing an install should be able to tell you, so it gets the first tab
 *  instead of a line buried under the API keys. */
export function UserIdentityStore() {
  const { userPreferences, preferenceError, setUserDisplayName } = useCompanionStore(
    useShallow((state) => ({
      userPreferences: state.userPreferences,
      preferenceError: state.preferenceError,
      setUserDisplayName: state.setUserDisplayName,
    })),
  );

  const storedName = userPreferences.displayName ?? "";
  const [name, setName] = useState(storedName);
  const [isSaving, setIsSaving] = useState(false);
  const [hasSaved, setHasSaved] = useState(false);

  // Preferences arrive after the first paint (and can change from elsewhere —
  // the store listens on preferences://changed), so the field follows the
  // stored value rather than freezing at whatever it was mounted with.
  // Deliberately NOT resetting `hasSaved` here: this effect fires as a result
  // of our own successful save, and clearing the flag would erase the "Saved"
  // confirmation in the same frame it appeared.
  useEffect(() => {
    setName(storedName);
  }, [storedName]);

  const trimmed = name.trim();
  const isDirty = trimmed !== storedName;

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!isDirty || isSaving) return;
    setIsSaving(true);
    try {
      await setUserDisplayName(trimmed);
      setHasSaved(true);
    } catch {
      // The store already published the message; keep the text in the field so
      // a refused name can be corrected rather than retyped.
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <section className="preference-store" aria-labelledby="user-identity-heading">
      <div>
        <p className="credential-store__eyebrow">You</p>
        <h2 id="user-identity-heading">Your name</h2>
        <p className="credential-store__description">
          What this app calls you. Leave it empty and it will simply say “You”.
        </p>
      </div>
      <form className="identity-form" onSubmit={handleSubmit}>
        <label className="credential-field credential-field--wide">
          <span>Display name</span>
          <input
            type="text"
            value={name}
            maxLength={60}
            placeholder="You"
            autoComplete="off"
            spellCheck={false}
            aria-label="Your display name"
            onChange={(event) => {
              setName(event.target.value);
              setHasSaved(false);
            }}
          />
        </label>
        <div className="identity-form__actions">
          {preferenceError ? (
            <small className="identity-form__error">{preferenceError}</small>
          ) : hasSaved ? (
            <small className="identity-form__saved" role="status">
              Saved
            </small>
          ) : null}
          <button
            className="credential-button credential-button--primary"
            type="submit"
            disabled={!isDirty || isSaving}
          >
            {isSaving ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </section>
  );
}
