import { useState, type ComponentType } from "react";

import { CompanionRoster } from "../features/companions/CompanionRoster";
import { StyleLibrary } from "../features/styles/StyleLibrary";
import { MemorySettingsSection } from "../features/memory/MemorySettingsSection";
import { ProviderApiKeyStore } from "../features/models/credentials/ProviderApiKeyStore";
import { ConfiguredModelStore } from "../features/models/configuredModels/ConfiguredModelStore";
import { UserPreferenceStore } from "../features/preferences/UserPreferenceStore";

interface SettingsTab {
  id: string;
  label: string;
  Panel: ComponentType;
}

function ModelsPanel() {
  return (
    <>
      <UserPreferenceStore />
      <ProviderApiKeyStore />
      <ConfiguredModelStore />
      <MemorySettingsSection />
    </>
  );
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
  { id: "models", label: "Models", Panel: ModelsPanel },
  { id: "companions", label: "Companions", Panel: CompanionsPanel },
  { id: "styles", label: "Styles", Panel: StylesPanel },
];

export function SettingsScreen() {
  const [activeTabId, setActiveTabId] = useState(SETTINGS_TABS[0].id);
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
