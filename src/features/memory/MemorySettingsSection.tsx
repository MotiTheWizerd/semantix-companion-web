// Settings — the Semantix Memory section: connect the account token that
// opens the memory organ, and the reflex toggles (master + one per sense).
// Reuses the credential-store styling so it sits beside the API-key card.

import { useEffect, useState, type FormEvent } from "react";

import {
  clearAccountToken,
  loadAccountToken,
  saveAccountToken,
} from "./organService";
import {
  isMemoryEnabled,
  memoryPrefs,
  PREF_MEMORY_ENABLED,
  reflexPrefKey,
  reflexSetting,
  setMemoryPref,
} from "./prefs";
import { memoryReflexes } from "./reflexes/registry";

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Companion could not update your memory settings.";
}

export function MemorySettingsSection() {
  const [isLoading, setIsLoading] = useState(true);
  const [isConnected, setIsConnected] = useState(false);
  const [token, setToken] = useState("");
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [prefs, setPrefs] = useState(() =>
    memoryPrefs(memoryReflexes.map((reflex) => reflex.id)),
  );

  useEffect(() => {
    let cancelled = false;
    void loadAccountToken()
      .then((saved) => {
        if (!cancelled) setIsConnected(Boolean(saved));
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const refreshPrefs = () =>
    setPrefs(memoryPrefs(memoryReflexes.map((reflex) => reflex.id)));

  const handleConnect = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!token.trim()) return;
    setError(null);
    setIsSaving(true);
    try {
      await saveAccountToken(token);
      setToken("");
      setIsConnected(true);
    } catch (saveError) {
      setError(errorMessage(saveError));
    } finally {
      setIsSaving(false);
    }
  };

  const handleDisconnect = async () => {
    setError(null);
    setIsSaving(true);
    try {
      await clearAccountToken();
      setIsConnected(false);
    } catch (clearError) {
      setError(errorMessage(clearError));
    } finally {
      setIsSaving(false);
    }
  };

  const memoryOn = isMemoryEnabled(prefs);

  return (
    <section className="credential-store" aria-labelledby="memory-settings-title">
      <div className="credential-store__header">
        <div>
          <h2 id="memory-settings-title">Semantix Memory</h2>
          <p>
            Give Companion a long-term memory: recall rides ahead of every message,
            and <code>/sleep</code> distills a conversation into memories.
          </p>
        </div>
      </div>

      {isLoading ? (
        <div className="credential-list__status">Checking memory account…</div>
      ) : isConnected ? (
        <div className="credential-form__fields">
          <label className="credential-field credential-field--wide">
            <span>Account</span>
            <div className="memory-settings__connected">
              <code>Connected — memories live on your Semantix account</code>
              <button
                type="button"
                className="credential-button credential-button--quiet"
                disabled={isSaving}
                onClick={() => void handleDisconnect()}
              >
                Disconnect
              </button>
            </div>
          </label>
        </div>
      ) : (
        <form className="credential-form" onSubmit={handleConnect}>
          <div className="credential-form__fields">
            <label className="credential-field credential-field--wide">
              <span>Semantix API token</span>
              <input
                type="password"
                autoComplete="off"
                spellCheck="false"
                required
                value={token}
                placeholder="sxa_…"
                onChange={(event) => setToken(event.target.value)}
              />
            </label>
          </div>
          <div className="credential-form__actions">
            <button
              type="submit"
              className="credential-button credential-button--primary"
              disabled={isSaving || !token.trim()}
            >
              {isSaving ? "Connecting…" : "Connect"}
            </button>
          </div>
        </form>
      )}

      {error ? (
        <p className="credential-store__error" role="alert">
          {error}
        </p>
      ) : null}

      {isConnected ? (
        <div className="credential-form__fields memory-settings__toggles">
          <label className="credential-field memory-settings__toggle">
            <input
              type="checkbox"
              checked={memoryOn}
              onChange={(event) => {
                setMemoryPref(PREF_MEMORY_ENABLED, event.target.checked);
                refreshPrefs();
              }}
            />
            <span>
              <strong>Memory</strong>
              <small>The master switch — off means the organ is never spoken to</small>
            </span>
          </label>
          {memoryReflexes.map((reflex) => (
            <label className="credential-field memory-settings__toggle" key={reflex.id}>
              <input
                type="checkbox"
                disabled={!memoryOn}
                checked={reflexSetting(prefs, reflex).enabled}
                onChange={(event) => {
                  setMemoryPref(reflexPrefKey(reflex.id), event.target.checked);
                  refreshPrefs();
                }}
              />
              <span>
                <strong>{reflex.label}</strong>
                <small>{reflex.description}</small>
              </span>
            </label>
          ))}
        </div>
      ) : null}

      <div className="credential-store__security-note">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 3 5.5 6v5.2c0 4.1 2.7 7.9 6.5 9.8 3.8-1.9 6.5-5.7 6.5-9.8V6L12 3Z" />
          <path d="m9.2 12 1.8 1.8 3.8-4" />
        </svg>
        <span>
          The token stays on this computer in the native system keychain. Recall only
          runs while a token is connected.
        </span>
      </div>
    </section>
  );
}
