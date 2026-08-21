import { useState } from "react";

import { AppHeader } from "./components/AppHeader";
import { AppSidebar } from "./components/AppSidebar";
import { ChatSurface } from "./components/ChatSurface";
import { SettingsScreen } from "./components/SettingsScreen";
import { useChatController } from "./features/chat/useChatController";

export function App() {
  const [activeView, setActiveView] = useState<"chat" | "settings">("chat");
  const chat = useChatController();
  const isSettings = activeView === "settings";

  const showChat = () => setActiveView("chat");
  const startNewConversation = () => {
    chat.startNewConversation();
    showChat();
  };
  const selectConversation = (conversationId: string) => {
    void chat.selectConversation(conversationId);
    showChat();
  };

  return (
    <div className="app-shell">
      <AppSidebar
        activeView={activeView}
        conversations={chat.conversations}
        activeConversationId={chat.activeConversationId}
        onViewChange={setActiveView}
        onNewConversation={startNewConversation}
        onConversationSelect={selectConversation}
      />
      <div className="app-workspace">
        <AppHeader
          eyebrow={isSettings ? "Companion" : "Conversation"}
          title={isSettings ? "Settings" : (chat.activeConversation?.title ?? "New conversation")}
          showOptions={!isSettings}
        />
        {isSettings ? (
          <SettingsScreen />
        ) : (
          <ChatSurface
            activeConversationId={chat.activeConversationId}
            messages={chat.messages}
            isLoading={chat.isLoading}
            isSending={chat.isSending}
            error={chat.error}
            configuredModels={chat.configuredModels}
            selectedModelId={chat.selectedModelId}
            onModelSelect={chat.selectModel}
            onSend={chat.send}
          />
        )}
      </div>
    </div>
  );
}
