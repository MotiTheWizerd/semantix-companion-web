import { lazy, Suspense, useCallback, useEffect } from "react";
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
    settlingTitles,
    companions,
    activeTab,
    activeTabId,
    runtime,
    isSubmitting,
    initialise,
    setActiveView,
    openConversation,
    openNewConversation,
    openSettings,
    setDraft,
    setTabCompanion,
    sendMessage,
    stopTurn,
    attachFiles,
    removeAttachment,
    userPreferences,
  } = useCompanionStore(
    // ⚑ THE SHELL READS ONLY THE ACTIVE THREAD. Selecting the whole runtime
    // map meant every streamed token — in any tab — replaced the map and
    // re-rendered the shell top to bottom. Now a token in the active thread
    // changes `runtime` and nothing else; a token in a background tab changes
    // nothing here at all.
    useShallow((state) => {
      const activeTab = state.activeTabId ? (state.tabsById[state.activeTabId] ?? null) : null;
      const activeConversationId = activeTab?.conversationId ?? null;
      return {
        activeView: state.activeView,
        isInitialising: state.isInitialising,
        conversations: state.conversations,
        settlingTitles: state.settlingTitles,
        companions: state.companions,
        activeTab,
        activeTabId: state.activeTabId,
        runtime: activeConversationId
          ? state.runtimeByConversationId[activeConversationId]
          : undefined,
        isSubmitting: Boolean(state.activeTabId && state.submittingByTabId[state.activeTabId]),
        initialise: state.initialise,
        setActiveView: state.setActiveView,
        openConversation: state.openConversation,
        openNewConversation: state.openNewConversation,
        openSettings: state.openSettings,
        setDraft: state.setDraft,
        setTabCompanion: state.setTabCompanion,
        sendMessage: state.sendMessage,
        stopTurn: state.stopTurn,
        attachFiles: state.attachFiles,
        removeAttachment: state.removeAttachment,
        userPreferences: state.userPreferences,
      };
    }),
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

  // Settings is a tab, not a view (s573): it shows when its tab is the active
  // one, with the strip above it like any conversation, so the way back is
  // one click and the way there did not close anything.
  const isSky = activeView === "memory";
  const isSettings = !isSky && activeTab?.kind === "settings";
  const activeConversationId = activeTab?.conversationId ?? null;
  const isSending = isSubmitting || Boolean(runtime?.isStreaming);

  // Stable across renders so the memoized shell pieces (sidebar, header, tab
  // strip) are not handed a fresh function per streamed frame.
  const handleConversationSelect = useCallback(
    (conversationId: string) => void openConversation(conversationId),
    [openConversation],
  );
  const handleContentChange = useCallback(
    (content: string) => {
      if (activeTabId) setDraft(activeTabId, content);
    },
    [activeTabId, setDraft],
  );
  const handleCompanionChange = useCallback(
    (companionId: string) => {
      if (activeTabId) void setTabCompanion(activeTabId, companionId);
    },
    [activeTabId, setTabCompanion],
  );
  const handleSend = useCallback(
    async (content: string) => {
      if (activeTabId) await sendMessage(activeTabId, content);
    },
    [activeTabId, sendMessage],
  );
  const handleStop = useCallback(() => {
    if (activeTabId) void stopTurn(activeTabId);
  }, [activeTabId, stopTurn]);
  const handleAttachFiles = useCallback(
    (files: File[]) => {
      if (activeTabId) void attachFiles(activeTabId, files);
    },
    [activeTabId, attachFiles],
  );
  const handleRemoveAttachment = useCallback(
    (attachmentId: string) => {
      if (activeTabId) removeAttachment(activeTabId, attachmentId);
    },
    [activeTabId, removeAttachment],
  );

  return (
    <div className={`app-shell${isSky ? " is-sky" : ""}`}>
      {/* The notification pulse line — every view, above everything. */}
      <NotificationStack />
      <AppSidebar
        activeView={activeView}
        conversations={conversations}
        settlingTitles={settlingTitles}
        activeConversationId={activeConversationId}
        userPreferences={userPreferences}
        isInitialising={isInitialising}
        isSettingsOpen={isSettings}
        onViewChange={setActiveView}
        onOpenSettings={openSettings}
        onNewConversation={openNewConversation}
        onConversationSelect={handleConversationSelect}
      />
      <div className={`app-workspace ${isSky ? "is-sky" : "has-conversation-tabs"}`}>
        {isSky ? null : (
          <AppHeader
            eyebrow={isSettings ? "Companion" : "Conversation"}
            title={isSettings ? "Settings" : (activeTab?.title ?? "New conversation")}
            showOptions={!isSettings}
          />
        )}
        {isSky ? null : <ConversationTabs />}
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
            thinkingMessageId={runtime?.thinkingMessageId ?? null}
            content={activeTab?.draft ?? ""}
            pendingAttachments={activeTab?.attachments ?? []}
            companions={companions}
            companionId={activeTab?.companionId ?? null}
            onContentChange={handleContentChange}
            onCompanionChange={handleCompanionChange}
            onSend={handleSend}
            onStop={handleStop}
            onAttachFiles={handleAttachFiles}
            onRemoveAttachment={handleRemoveAttachment}
          />
        )}
      </div>
    </div>
  );
}
