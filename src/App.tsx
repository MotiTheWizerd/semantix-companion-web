import { useEffect } from "react";
import { useShallow } from "zustand/react/shallow";

import { AppHeader } from "./components/AppHeader";
import { AppSidebar } from "./components/AppSidebar";
import { ChatSurface } from "./components/ChatSurface";
import { NotificationStack } from "./components/NotificationStack";
import { ConversationTabs } from "./components/ConversationTabs";
import { SettingsScreen } from "./components/SettingsScreen";
import { useCompanionStore } from "./features/workspace/companionStore";

export function App() {
  const {
    activeView,
    isInitialising,
    conversations,
    configuredModels,
    userPreferences,
    tabsById,
    activeTabId,
    runtimeByConversationId,
    submittingByTabId,
    initialise,
    setActiveView,
    openConversation,
    openNewConversation,
    setDraft,
    setTabModelPreference,
    sendMessage,
  } = useCompanionStore(
    useShallow((state) => ({
      activeView: state.activeView,
      isInitialising: state.isInitialising,
      conversations: state.conversations,
      configuredModels: state.configuredModels,
      userPreferences: state.userPreferences,
      tabsById: state.tabsById,
      activeTabId: state.activeTabId,
      runtimeByConversationId: state.runtimeByConversationId,
      submittingByTabId: state.submittingByTabId,
      initialise: state.initialise,
      setActiveView: state.setActiveView,
      openConversation: state.openConversation,
      openNewConversation: state.openNewConversation,
      setDraft: state.setDraft,
      setTabModelPreference: state.setTabModelPreference,
      sendMessage: state.sendMessage,
    })),
  );

  useEffect(() => {
    void initialise();
  }, [initialise]);

  const isSettings = activeView === "settings";
  const activeTab = activeTabId ? tabsById[activeTabId] : null;
  const activeConversationId = activeTab?.conversationId ?? null;
  const runtime = activeConversationId
    ? runtimeByConversationId[activeConversationId]
    : undefined;
  const isSending = Boolean(
    (activeTabId && submittingByTabId[activeTabId]) || runtime?.isStreaming,
  );

  return (
    <div className="app-shell">
      {/* The notification pulse line — every view, above everything. */}
      <NotificationStack />
      <AppSidebar
        activeView={activeView}
        conversations={conversations}
        activeConversationId={activeConversationId}
        onViewChange={setActiveView}
        onNewConversation={openNewConversation}
        onConversationSelect={(conversationId) => void openConversation(conversationId)}
      />
      <div className={`app-workspace ${isSettings ? "" : "has-conversation-tabs"}`}>
        <AppHeader
          eyebrow={isSettings ? "Companion" : "Conversation"}
          title={isSettings ? "Settings" : (activeTab?.title ?? "New conversation")}
          showOptions={!isSettings}
        />
        {isSettings ? null : <ConversationTabs />}
        {isSettings ? (
          <SettingsScreen />
        ) : (
          <ChatSurface
            activeConversationId={activeConversationId}
            messages={runtime?.messages ?? []}
            isLoading={isInitialising || Boolean(runtime?.isLoading)}
            isSending={isSending}
            error={activeTab?.error ?? runtime?.error ?? null}
            notice={activeTab?.notice ?? null}
            recallByMessageId={runtime?.recallByMessageId ?? {}}
            toolCallsByMessageId={runtime?.toolCallsByMessageId ?? {}}
            content={activeTab?.draft ?? ""}
            configuredModels={configuredModels}
            modelPreference={activeTab?.modelPreference ?? { mode: "inherit" }}
            userPreferences={userPreferences}
            onContentChange={(content) => {
              if (activeTabId) setDraft(activeTabId, content);
            }}
            onModelPreferenceChange={(preference) => {
              if (activeTabId) void setTabModelPreference(activeTabId, preference);
            }}
            onSend={async (content) => {
              if (activeTabId) await sendMessage(activeTabId, content);
            }}
          />
        )}
      </div>
    </div>
  );
}
