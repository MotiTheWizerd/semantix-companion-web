import { useShallow } from "zustand/react/shallow";

import { useCompanionStore } from "../features/workspace/companionStore";

function CloseIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="m4.5 4.5 7 7M11.5 4.5l-7 7" />
    </svg>
  );
}

function PlusIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M8 3.5v9M3.5 8h9" />
    </svg>
  );
}

export function ConversationTabs() {
  const { tabOrder, tabsById, activeTabId, setActiveTab, closeTab, openNewConversation } =
    useCompanionStore(
      useShallow((state) => ({
        tabOrder: state.tabOrder,
        tabsById: state.tabsById,
        activeTabId: state.activeTabId,
        setActiveTab: state.setActiveTab,
        closeTab: state.closeTab,
        openNewConversation: state.openNewConversation,
      })),
    );

  return (
    <div className="conversation-tabs" role="tablist" aria-label="Open conversations">
      <div className="conversation-tabs__track">
        {tabOrder.map((tabId) => {
          const tab = tabsById[tabId];
          if (!tab) return null;
          const isActive = tabId === activeTabId;
          return (
            <div className={`conversation-tab ${isActive ? "is-active" : ""}`} key={tabId}>
              <button
                className="conversation-tab__select"
                type="button"
                role="tab"
                aria-selected={isActive}
                title={tab.title}
                onClick={() => setActiveTab(tabId)}
              >
                {tab.unreadCount > 0 ? <span className="conversation-tab__unread" /> : null}
                <span>{tab.title}</span>
              </button>
              <button
                className="conversation-tab__close"
                type="button"
                aria-label={`Close ${tab.title}`}
                onClick={() => closeTab(tabId)}
              >
                <CloseIcon />
              </button>
            </div>
          );
        })}
      </div>
      <button
        className="conversation-tabs__add"
        type="button"
        aria-label="Open new conversation tab"
        onClick={openNewConversation}
      >
        <PlusIcon />
      </button>
    </div>
  );
}
