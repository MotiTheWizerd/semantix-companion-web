import { lazy, Suspense, useEffect } from "react";
import { useShallow } from "zustand/react/shallow";

import { AppHeader } from "./components/AppHeader";
import { AppSidebar } from "./components/AppSidebar";
import { ChatSurface } from "./components/ChatSurface";
import { NotificationStack } from "./components/NotificationStack";
import { ConversationTabs } from "./components/ConversationTabs";
import { SettingsScreen } from "./components/SettingsScreen";
import { useUiZoom } from "./features/appearance/useUiZoom";
import { bindImportNotifications } from "./features/import/importNotifications";

// The sky carries three.js (~600 kB); the chat never pays for it.
const MemorySkyView = lazy(() =>
  import("./features/memory-sky/MemorySkyView").then((m) => ({ default: m.MemorySkyView })),
);
import { useCompanionStore } from "./features/workspace/companionStore";

export function App() {
  const {
    activeView,
    isInitialising,
    conversations,
    companions,
    tabsById,
    activeTabId,
    runtimeByConversationId,
    submittingByTabId,
    initialise,
    setActiveView,
    openConversation,
    openNewConversation,
    setDraft,
    setTabCompanion,
    sendMessage,
    stopTurn,
    attachFiles,
    removeAttachment,
  } = useCompanionStore(
    useShallow((state) => ({
      activeView: state.activeView,
      isInitialising: state.isInitialising,
      conversations: state.conversations,
      companions: state.companions,
      tabsById: state.tabsById,
      activeTabId: state.activeTabId,
      runtimeByConversationId: state.runtimeByConversationId,
      submittingByTabId: state.submittingByTabId,
      initialise: state.initialise,
      setActiveView: state.setActiveView,
      openConversation: state.openConversation,
      openNewConversation: state.openNewConversation,
      setDraft: state.setDraft,
      setTabCompanion: state.setTabCompanion,
      sendMessage: state.sendMessage,
      stopTurn: state.stopTurn,
      attachFiles: state.attachFiles,
      removeAttachment: state.removeAttachment,
    })),
  );

  // Ctrl/Cmd +/- for the whole interface. Bound at the shell so it answers
  // from any view, and restored from the last run before the first paint.
  useUiZoom();

  useEffect(() => {
    void initialise();
  }, [initialise]);

  // A history import narrates itself through the notification stack for the
  // life of the app, wherever the user wanders — bound once, at the shell.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void bindImportNotifications().then((stop) => {
      if (cancelled) stop();
      else unlisten = stop;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const isSettings = activeView === "settings";
  const isSky = activeView === "memory";
  const activeTab = activeTabId ? tabsById[activeTabId] : null;
  const activeConversationId = activeTab?.conversationId ?? null;
  const runtime = activeConversationId
    ? runtimeByConversationId[activeConversationId]
    : undefined;
  const isSending = Boolean(
    (activeTabId && submittingByTabId[activeTabId]) || runtime?.isStreaming,
  );

  return (
    <div className={`app-shell${isSky ? " is-sky" : ""}`}>
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
      <div
        className={`app-workspace ${isSky ? "is-sky" : isSettings ? "" : "has-conversation-tabs"}`}
      >
        {isSky ? null : (
          <AppHeader
            eyebrow={isSettings ? "Companion" : "Conversation"}
            title={isSettings ? "Settings" : (activeTab?.title ?? "New conversation")}
            showOptions={!isSettings}
          />
        )}
        {isSettings || isSky ? null : <ConversationTabs />}
        {isSky ? (
          <Suspense fallback={<div className="memory-sky" />}>
            <MemorySkyView
              companions={companions}
              initialCompanionId={activeTab?.companionId ?? null}
            />
          </Suspense>
        ) : isSettings ? (
          <SettingsScreen />
        ) : (
          <ChatSurface
            activeConversationId={activeConversationId}
            messages={runtime?.messages ?? []}
            isLoading={isInitialising || Boolean(runtime?.isLoading)}
            isSending={isSending}
            isRemembering={Boolean(runtime?.isRemembering)}
            error={activeTab?.error ?? runtime?.error ?? null}
            notice={activeTab?.notice ?? null}
            recallByMessageId={runtime?.recallByMessageId ?? {}}
            toolCallsByMessageId={runtime?.toolCallsByMessageId ?? {}}
            reasoningByMessageId={runtime?.reasoningByMessageId ?? {}}
            content={activeTab?.draft ?? ""}
            pendingAttachments={activeTab?.attachments ?? []}
            companions={companions}
            companionId={activeTab?.companionId ?? null}
            onContentChange={(content) => {
              if (activeTabId) setDraft(activeTabId, content);
            }}
            onCompanionChange={(companionId) => {
              if (activeTabId) void setTabCompanion(activeTabId, companionId);
            }}
            onSend={async (content) => {
              if (activeTabId) await sendMessage(activeTabId, content);
            }}
            onStop={() => {
              if (activeTabId) void stopTurn(activeTabId);
            }}
            onAttachFiles={(files) => {
              if (activeTabId) void attachFiles(activeTabId, files);
            }}
            onRemoveAttachment={(attachmentId) => {
              if (activeTabId) removeAttachment(activeTabId, attachmentId);
            }}
          />
        )}
      </div>
    </div>
  );
}
