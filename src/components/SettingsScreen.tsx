import { MemorySettingsSection } from "../features/memory/MemorySettingsSection";
import { ProviderApiKeyStore } from "../features/models/credentials/ProviderApiKeyStore";
import { ConfiguredModelStore } from "../features/models/configuredModels/ConfiguredModelStore";
import { UserPreferenceStore } from "../features/preferences/UserPreferenceStore";

const SETTINGS_TABS = [{ id: "models", label: "Models" }] as const;

export function SettingsScreen() {
  return (
    <main className="settings-screen" aria-labelledby="settings-title">
      <div className="settings-screen__inner">
        <div className="settings-screen__heading">
          <p>Companion</p>
          <h1 id="settings-title">Settings</h1>
        </div>

        <div className="settings-tabs" role="tablist" aria-label="Settings sections">
          {SETTINGS_TABS.map((tab) => (
            <button
              className="settings-tab is-active"
              id={`settings-tab-${tab.id}`}
              key={tab.id}
              type="button"
              role="tab"
              aria-selected="true"
              aria-controls={`settings-panel-${tab.id}`}
            >
              {tab.label}
            </button>
          ))}
        </div>

        <section
          className="settings-panel"
          id="settings-panel-models"
          role="tabpanel"
          aria-labelledby="settings-tab-models"
        >
          <UserPreferenceStore />
          <ProviderApiKeyStore />
          <ConfiguredModelStore />
          <MemorySettingsSection />
        </section>
      </div>
    </main>
  );
}
