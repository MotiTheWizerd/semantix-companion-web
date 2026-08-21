import { useEffect, useMemo, useState, type FormEvent } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import { ConfirmDeleteButton } from "../../../components/ConfirmDeleteButton";
import { EditButton } from "../../../components/EditButton";
import {
  createProviderCredential,
  deleteProviderCredential,
  listKnownModelProviders,
  listProviderCredentials,
  onCredentialsChanged,
  updateProviderCredential,
} from "./credentialService";
import type {
  CredentialChangedEvent,
  ModelProvider,
  ProviderCredential,
} from "./types";

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Companion could not access your saved API keys.";
}

function reconcileCredentialEvent(
  credentials: ProviderCredential[],
  event: CredentialChangedEvent,
): ProviderCredential[] {
  if (event.kind === "deleted") {
    return credentials.filter((credential) => credential.id !== event.credentialId);
  }

  return [
    event.credential,
    ...credentials.filter((credential) => credential.id !== event.credential.id),
  ].sort((left, right) => right.updatedAt - left.updatedAt);
}

export function ProviderApiKeyStore() {
  const [providers, setProviders] = useState<ModelProvider[]>([]);
  const [credentials, setCredentials] = useState<ProviderCredential[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isAdding, setIsAdding] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [providerId, setProviderId] = useState("");
  const [label, setLabel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);

  const selectedProvider = useMemo(
    () => providers.find((provider) => provider.id === providerId),
    [providerId, providers],
  );
  const providerNames = useMemo(
    () => new Map(providers.map((provider) => [provider.id, provider.name])),
    [providers],
  );
  const isEditing = editingId !== null;
  const canSave =
    providerId.length > 0 &&
    (isEditing
      ? apiKey.trim().length === 0 || apiKey.trim().length >= 8
      : apiKey.trim().length >= 8);

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;

    const initialise = async () => {
      try {
        unlisten = await onCredentialsChanged((event) => {
          if (!cancelled) {
            setCredentials((current) => reconcileCredentialEvent(current, event));
          }
        });

        const [knownProviders, savedCredentials] = await Promise.all([
          listKnownModelProviders(),
          listProviderCredentials(),
        ]);
        if (cancelled) return;

        setProviders(knownProviders);
        setProviderId((current) => current || knownProviders[0]?.id || "");
        setCredentials(savedCredentials);
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

  const resetForm = () => {
    setLabel("");
    setApiKey("");
    setError(null);
  };

  const closeForm = () => {
    resetForm();
    setEditingId(null);
    setIsAdding(false);
  };

  const openCreateForm = () => {
    resetForm();
    setEditingId(null);
    setProviderId(providers[0]?.id ?? "");
    setIsAdding(true);
  };

  const openEditForm = (credential: ProviderCredential) => {
    resetForm();
    setEditingId(credential.id);
    setProviderId(credential.providerId);
    setLabel(credential.label);
    setPendingDeleteId(null);
    setIsAdding(true);
  };

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSave) return;

    setError(null);
    setIsSaving(true);
    try {
      const credential = isEditing
        ? await updateProviderCredential({
            credentialId: editingId,
            providerId,
            label,
            apiKey: apiKey.trim() || null,
          })
        : await createProviderCredential({
            providerId,
            label,
            apiKey,
          });
      setCredentials((current) =>
        reconcileCredentialEvent(current, {
          kind: isEditing ? "updated" : "created",
          credential,
        }),
      );
      closeForm();
    } catch (saveError) {
      setError(errorMessage(saveError));
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async (credentialId: string) => {
    if (pendingDeleteId !== credentialId) {
      setPendingDeleteId(credentialId);
      return;
    }

    setError(null);
    setDeletingId(credentialId);
    try {
      await deleteProviderCredential(credentialId);
      setCredentials((current) =>
        reconcileCredentialEvent(current, { kind: "deleted", credentialId }),
      );
      if (editingId === credentialId) closeForm();
      setPendingDeleteId(null);
    } catch (deleteError) {
      setError(errorMessage(deleteError));
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <section className="credential-store" aria-labelledby="saved-api-keys-title">
      <div className="credential-store__header">
        <div>
          <h2 id="saved-api-keys-title">Saved API keys</h2>
          <p>Save a provider key once, then reuse it when configuring models.</p>
        </div>
        <button
          className="credential-store__add-button"
          type="button"
          aria-expanded={isAdding}
          onClick={() => (isAdding ? closeForm() : openCreateForm())}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 5v14M5 12h14" />
          </svg>
          Add API key
        </button>
      </div>

      {isAdding ? (
        <form className="credential-form" onSubmit={handleSubmit}>
          <div className="credential-form__heading">
            <div>
              <h3>{isEditing ? "Edit provider key" : "Add provider key"}</h3>
              <p>
                {isEditing
                  ? "Change its metadata or enter a new secret to rotate the saved key."
                  : "The secret is stored by your operating system, not in Companion's database."}
              </p>
            </div>
            <button
              className="credential-form__close"
              type="button"
              aria-label={`Close ${isEditing ? "edit" : "add"} API key form`}
              onClick={closeForm}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="m7 7 10 10M17 7 7 17" />
              </svg>
            </button>
          </div>

          <div className="credential-form__fields">
            <label className="credential-field">
              <span>Provider</span>
              <select
                value={providerId}
                required
                onChange={(event) => setProviderId(event.target.value)}
              >
                {providers.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.name}
                  </option>
                ))}
              </select>
            </label>

            <label className="credential-field">
              <span>Label <small>Optional</small></span>
              <input
                type="text"
                maxLength={80}
                value={label}
                placeholder={selectedProvider ? `${selectedProvider.name} key` : "Personal key"}
                onChange={(event) => setLabel(event.target.value)}
              />
            </label>

            <label className="credential-field credential-field--wide">
              <span>
                API key {isEditing ? <small>Leave blank to keep current</small> : null}
              </span>
              <input
                type="password"
                autoComplete="off"
                spellCheck="false"
                required={!isEditing}
                minLength={8}
                value={apiKey}
                placeholder={
                  isEditing
                    ? "Leave blank to keep the saved secret"
                    : (selectedProvider?.keyPlaceholder ?? "Enter API key")
                }
                onChange={(event) => setApiKey(event.target.value)}
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
              disabled={isSaving || !canSave}
            >
              {isSaving ? "Saving…" : isEditing ? "Save changes" : "Save API key"}
            </button>
          </div>
        </form>
      ) : null}

      {!isAdding && error ? (
        <p className="credential-store__error" role="alert">
          {error}
        </p>
      ) : null}

      <div className="credential-list" aria-live="polite" aria-busy={isLoading}>
        {isLoading ? (
          <div className="credential-list__status">Loading saved keys…</div>
        ) : credentials.length === 0 ? (
          <div className="credential-list__empty">
            <div className="credential-list__empty-icon" aria-hidden="true">
              <svg viewBox="0 0 24 24">
                <circle cx="8" cy="15" r="3" />
                <path d="m10.5 13.5 7-7M15 9l2 2M17 7l2 2" />
              </svg>
            </div>
            <strong>No saved API keys</strong>
            <span>Add a provider key to make it available to your model configurations.</span>
          </div>
        ) : (
          <ul>
            {credentials.map((credential) => (
              <li key={credential.id}>
                <div className="credential-list__provider" aria-hidden="true">
                  {(providerNames.get(credential.providerId) ?? credential.providerId)
                    .charAt(0)
                    .toUpperCase()}
                </div>
                <div className="credential-list__identity">
                  <strong>{credential.label}</strong>
                  <span>{providerNames.get(credential.providerId) ?? credential.providerId}</span>
                </div>
                <code>{credential.keyHint}</code>
                <div className="credential-list__actions">
                  <EditButton
                    label={credential.label}
                    onClick={() => openEditForm(credential)}
                  />
                  <ConfirmDeleteButton
                    label={credential.label}
                    isConfirming={pendingDeleteId === credential.id}
                    isDeleting={deletingId === credential.id}
                    onRequestConfirmation={() => setPendingDeleteId(credential.id)}
                    onCancel={() => setPendingDeleteId(null)}
                    onConfirm={() => void handleDelete(credential.id)}
                  />
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="credential-store__security-note">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 3 5.5 6v5.2c0 4.1 2.7 7.9 6.5 9.8 3.8-1.9 6.5-5.7 6.5-9.8V6L12 3Z" />
          <path d="m9.2 12 1.8 1.8 3.8-4" />
        </svg>
        <span>
          Keys stay on this computer in the native system keychain. Saved secrets are never
          displayed again.
        </span>
      </div>
    </section>
  );
}
