import { useState, type ComponentType } from "react";

import { CompanionRoster } from "../features/companions/CompanionRoster";
import { StyleLibrary } from "../features/styles/StyleLibrary";
import { MemorySettingsSection } from "../features/memory/MemorySettingsSection";
import { ProviderApiKeyStore } from "../features/models/credentials/ProviderApiKeyStore";
import { ConfiguredModelStore } from "../features/models/configuredModels/ConfiguredModelStore";
import { UserPreferenceStore } from "../features/preferences/UserPreferenceStore";
import { UserIdentityStore } from "../features/preferences/UserIdentityStore";

interface SettingsTab {
  id: string;
  label: string;
  Panel: ComponentType;
}

function YouPanel() {
  return <UserIdentityStore />;
}

function ModelsPanel() {
  return (
    <>
      <UserPreferenceStore />
      <ConfiguredModelStore />
      <MemorySettingsSection />
    </>
  );
}

/** The keys live apart from the models that use them: a key is a credential
 *  with a provider, a model is a choice — mixing the two on one tab made the
 *  Models tab the longest page in Settings. */
function ApiManagerPanel() {
  return <ProviderApiKeyStore />;
}

function CompanionsPanel() {
  return <CompanionRoster />;
}

function StylesPanel() {
  return <StyleLibrary />;
}

/** The registry drives both the tab strip and the panel — a new settings
 *  section is one entry here and nothing else. */
const SETTINGS_TABS: SettingsTab[] = [
  // First, because it is the one setting that is about the reader rather than
  // the machinery — and the one a fresh install most needs answered.
  { id: "you", label: "You", Panel: YouPanel },
  { id: "models", label: "Models", Panel: ModelsPanel },
  { id: "api-manager", label: "API Manager", Panel: ApiManagerPanel },
  { id: "companions", label: "Companions", Panel: CompanionsPanel },
  { id: "styles", label: "Styles", Panel: StylesPanel },
];

/** The section last opened, kept across mounts: Settings is a tab (s573),
 *  and the screen unmounts whenever another tab is up. Coming back from a
 *  conversation should land where the reader left — on the API Manager,
 *  not reset to "You". Not persisted; a fresh launch starts at the top. */
let lastSectionId = SETTINGS_TABS[0].id;

export function SettingsScreen() {
  const [activeTabId, setActiveTabState] = useState(lastSectionId);
  const setActiveTabId = (id: string) => {
    lastSectionId = id;
    setActiveTabState(id);
  };
  const activeTab =
    SETTINGS_TABS.find((tab) => tab.id === activeTabId) ?? SETTINGS_TABS[0];
  const { Panel } = activeTab;

  return (
    <main className="settings-screen" aria-labelledby="settings-title">
      <div className="settings-screen__inner">
        <div className="settings-screen__heading">
          <p>Companion</p>
          <h1 id="settings-title">Settings</h1>
        </div>

        <div className="settings-tabs" role="tablist" aria-label="Settings sections">
          {SETTINGS_TABS.map((tab) => {
            const isActive = tab.id === activeTab.id;
            return (
              <button
                className={isActive ? "settings-tab is-active" : "settings-tab"}
                id={`settings-tab-${tab.id}`}
                key={tab.id}
                type="button"
                role="tab"
                aria-selected={isActive}
                aria-controls={`settings-panel-${tab.id}`}
                onClick={() => setActiveTabId(tab.id)}
              >
                {tab.label}
              </button>
            );
          })}
        </div>

        <section
          className="settings-panel"
          id={`settings-panel-${activeTab.id}`}
          role="tabpanel"
          aria-labelledby={`settings-tab-${activeTab.id}`}
        >
          <Panel />
        </section>
      </div>
    </main>
  );
}
