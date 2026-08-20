import { useEffect, useMemo, useState, type FormEvent } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import {
  listKnownModelProviders,
  listProviderCredentials,
  onCredentialsChanged,
} from "../credentials/credentialService";
import type {
  CredentialChangedEvent,
  ModelProvider,
  ProviderCredential,
} from "../credentials/types";
import {
  createConfiguredModel,
  deleteConfiguredModel,
  listConfiguredModels,
  onModelsChanged,
} from "./modelService";
import type {
  ConfiguredModel,
  ModelChangedEvent,
  ModelCredentialKind,
} from "./types";

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Companion could not update your configured models.";
}

function reconcileModelEvent(
  models: ConfiguredModel[],
  event: ModelChangedEvent,
): ConfiguredModel[] {
  if (event.kind === "deleted") {
    return models.filter((model) => model.id !== event.modelId);
  }

  return [event.model, ...models.filter((model) => model.id !== event.model.id)].sort(
    (left, right) => right.updatedAt - left.updatedAt,
  );
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

export function ConfiguredModelStore() {
  const [providers, setProviders] = useState<ModelProvider[]>([]);
  const [credentials, setCredentials] = useState<ProviderCredential[]>([]);
  const [models, setModels] = useState<ConfiguredModel[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isAdding, setIsAdding] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [providerId, setProviderId] = useState("");
  const [modelId, setModelId] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [credentialKind, setCredentialKind] = useState<ModelCredentialKind>("saved");
  const [credentialId, setCredentialId] = useState("");
  const [manualApiKey, setManualApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);

  const providerNames = useMemo(
    () => new Map(providers.map((provider) => [provider.id, provider.name])),
    [providers],
  );
  const selectedProvider = useMemo(
    () => providers.find((provider) => provider.id === providerId),
    [providerId, providers],
  );
  const providerCredentials = useMemo(
    () => credentials.filter((credential) => credential.providerId === providerId),
    [credentials, providerId],
  );

  useEffect(() => {
    let cancelled = false;
    let unlistenModels: UnlistenFn | undefined;
    let unlistenCredentials: UnlistenFn | undefined;

    const initialise = async () => {
      try {
        [unlistenModels, unlistenCredentials] = await Promise.all([
          onModelsChanged((event) => {
            if (!cancelled) {
              setModels((current) => reconcileModelEvent(current, event));
            }
          }),
          onCredentialsChanged((event) => {
            if (!cancelled) {
              setCredentials((current) => reconcileCredentialEvent(current, event));
            }
          }),
        ]);

        const [knownProviders, savedCredentials, configuredModels] = await Promise.all([
          listKnownModelProviders(),
          listProviderCredentials(),
          listConfiguredModels(),
        ]);
        if (cancelled) return;

        setProviders(knownProviders);
        setProviderId((current) => current || knownProviders[0]?.id || "");
        setCredentials(savedCredentials);
        setModels(configuredModels);
      } catch (initialisationError) {
        if (!cancelled) setError(errorMessage(initialisationError));
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    };

    void initialise();
    return () => {
      cancelled = true;
      unlistenModels?.();
      unlistenCredentials?.();
    };
  }, []);

  useEffect(() => {
    if (providerCredentials.some((credential) => credential.id === credentialId)) return;
    setCredentialId(providerCredentials[0]?.id ?? "");
  }, [credentialId, providerCredentials]);

  const resetForm = () => {
    setModelId("");
    setDisplayName("");
    setManualApiKey("");
    setError(null);
  };

  const openForm = () => {
    resetForm();
    setCredentialKind(providerCredentials.length > 0 ? "saved" : "manual");
    setIsAdding(true);
  };

  const closeForm = () => {
    resetForm();
    setIsAdding(false);
  };

  const canSubmit =
    providerId.length > 0 &&
    modelId.trim().length > 0 &&
    (credentialKind === "saved" ? credentialId.length > 0 : manualApiKey.trim().length >= 8);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!canSubmit) return;

    setError(null);
    setIsSaving(true);
    try {
      const model = await createConfiguredModel({
        providerId,
        modelId,
        displayName,
        credential:
          credentialKind === "saved"
            ? { kind: "saved", credentialId }
            : { kind: "manual", apiKey: manualApiKey },
      });
      setModels((current) => reconcileModelEvent(current, { kind: "created", model }));
      closeForm();
    } catch (saveError) {
      setError(errorMessage(saveError));
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async (configuredModelId: string) => {
    if (pendingDeleteId !== configuredModelId) {
      setPendingDeleteId(configuredModelId);
      return;
    }

    setError(null);
    setDeletingId(configuredModelId);
    try {
      await deleteConfiguredModel(configuredModelId);
      setModels((current) =>
        reconcileModelEvent(current, { kind: "deleted", modelId: configuredModelId }),
      );
      setPendingDeleteId(null);
    } catch (deleteError) {
      setError(errorMessage(deleteError));
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <section className="model-store" aria-labelledby="configured-models-title">
      <div className="model-store__header">
        <div>
          <h2 id="configured-models-title">Models</h2>
          <p>Configure the models Companion can use and choose how each one authenticates.</p>
        </div>
        <button
          className="credential-store__add-button"
          type="button"
          aria-expanded={isAdding}
          onClick={() => (isAdding ? closeForm() : openForm())}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 5v14M5 12h14" />
          </svg>
          Add model
        </button>
      </div>

      {isAdding ? (
        <form className="credential-form model-form" onSubmit={handleSubmit}>
          <div className="credential-form__heading">
            <div>
              <h3>Add model</h3>
              <p>Use a reusable saved key or keep a manual key scoped only to this model.</p>
            </div>
            <button
              className="credential-form__close"
              type="button"
              aria-label="Close add model form"
              onClick={closeForm}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="m7 7 10 10M17 7 7 17" />
              </svg>
            </button>
          </div>

          <div className="credential-form__fields model-form__fields">
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
              <span>Model ID</span>
              <input
                type="text"
                maxLength={200}
                required
                value={modelId}
                placeholder="e.g. gpt-5"
                onChange={(event) => setModelId(event.target.value)}
              />
            </label>

            <label className="credential-field credential-field--wide">
              <span>
                Display name <small>Optional</small>
              </span>
              <input
                type="text"
                maxLength={100}
                value={displayName}
                placeholder={modelId.trim() || "Name shown inside Companion"}
                onChange={(event) => setDisplayName(event.target.value)}
              />
            </label>
          </div>

          <fieldset className="model-credential-source">
            <legend>API key</legend>
            <div className="model-credential-source__switch" role="radiogroup">
              <button
                className={credentialKind === "saved" ? "is-active" : ""}
                type="button"
                role="radio"
                aria-checked={credentialKind === "saved"}
                onClick={() => setCredentialKind("saved")}
              >
                Saved API key
              </button>
              <button
                className={credentialKind === "manual" ? "is-active" : ""}
                type="button"
                role="radio"
                aria-checked={credentialKind === "manual"}
                onClick={() => setCredentialKind("manual")}
              >
                Enter manually
              </button>
            </div>

            {credentialKind === "saved" ? (
              providerCredentials.length > 0 ? (
                <label className="credential-field model-credential-source__field">
                  <span>{selectedProvider?.name ?? "Provider"} keys</span>
                  <select
                    value={credentialId}
                    required
                    onChange={(event) => setCredentialId(event.target.value)}
                  >
                    {providerCredentials.map((credential) => (
                      <option key={credential.id} value={credential.id}>
                        {credential.label} — {credential.keyHint}
                      </option>
                    ))}
                  </select>
                </label>
              ) : (
                <div className="model-credential-source__empty">
                  <span>No saved {selectedProvider?.name ?? "provider"} key yet.</span>
                  <button type="button" onClick={() => setCredentialKind("manual")}>Use a manual key</button>
                </div>
              )
            ) : (
              <label className="credential-field model-credential-source__field">
                <span>Manual API key</span>
                <input
                  type="password"
                  autoComplete="off"
                  spellCheck="false"
                  required
                  minLength={8}
                  value={manualApiKey}
                  placeholder={selectedProvider?.keyPlaceholder ?? "Enter API key"}
                  onChange={(event) => setManualApiKey(event.target.value)}
                />
                <small>This key stays private to this model configuration.</small>
              </label>
            )}
          </fieldset>

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
              disabled={isSaving || !canSubmit}
            >
              {isSaving ? "Adding…" : "Add model"}
            </button>
          </div>
        </form>
      ) : null}

      {!isAdding && error ? (
        <p className="credential-store__error" role="alert">
          {error}
        </p>
      ) : null}

      <div className="model-list" aria-live="polite" aria-busy={isLoading}>
        {isLoading ? (
          <div className="model-list__status">Loading configured models…</div>
        ) : models.length === 0 ? (
          <div className="model-list__empty">
            <div className="model-list__empty-icon" aria-hidden="true">
              <svg viewBox="0 0 24 24">
                <path d="M12 3 4.5 7.2v8.6L12 21l7.5-5.2V7.2L12 3Z" />
                <path d="m4.8 7.4 7.2 4.4 7.2-4.4M12 11.8V21" />
              </svg>
            </div>
            <strong>No configured models</strong>
            <span>Add the first model Companion will be able to use.</span>
          </div>
        ) : (
          <ul>
            {models.map((model) => (
              <li key={model.id}>
                <div className="model-list__provider" aria-hidden="true">
                  {(providerNames.get(model.providerId) ?? model.providerId)
                    .charAt(0)
                    .toUpperCase()}
                </div>
                <div className="model-list__identity">
                  <strong>{model.displayName}</strong>
                  <span>
                    {providerNames.get(model.providerId) ?? model.providerId} · {model.modelId}
                  </span>
                </div>
                <div className="model-list__credential">
                  <span className={`model-list__source is-${model.credentialKind}`}>
                    {model.credentialKind === "saved" ? "Saved" : "Manual"}
                  </span>
                  <small>{model.credentialLabel} · {model.keyHint}</small>
                </div>
                <button
                  className={`credential-list__delete${pendingDeleteId === model.id ? " is-confirming" : ""}`}
                  type="button"
                  disabled={deletingId === model.id}
                  aria-label={
                    pendingDeleteId === model.id
                      ? `Confirm removal of ${model.displayName}`
                      : `Remove ${model.displayName}`
                  }
                  onBlur={() =>
                    setPendingDeleteId((current) => (current === model.id ? null : current))
                  }
                  onClick={() => void handleDelete(model.id)}
                >
                  {pendingDeleteId === model.id ? (
                    deletingId === model.id ? "Removing…" : "Remove?"
                  ) : (
                    <svg viewBox="0 0 24 24" aria-hidden="true">
                      <path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13M10 11v5M14 11v5" />
                    </svg>
                  )}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

