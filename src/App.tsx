import { useState } from "react";

import { AppHeader } from "./components/AppHeader";
import { AppSidebar } from "./components/AppSidebar";
import { ChatSurface } from "./components/ChatSurface";
import { SettingsScreen } from "./components/SettingsScreen";

export function App() {
  const [activeView, setActiveView] = useState<"chat" | "settings">("chat");
  const isSettings = activeView === "settings";

  return (
    <div className="app-shell">
      <AppSidebar activeView={activeView} onViewChange={setActiveView} />
      <div className="app-workspace">
        <AppHeader
          eyebrow={isSettings ? "Companion" : "Conversation"}
          title={isSettings ? "Settings" : "New conversation"}
          showOptions={!isSettings}
        />
        {isSettings ? <SettingsScreen /> : <ChatSurface />}
      </div>
    </div>
  );
}
