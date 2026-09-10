import { memo } from "react";
import { useShallow } from "zustand/react/shallow";

import { useCompanionStore } from "../features/workspace/companionStore";
import { CompanionMark } from "./CompanionMark";
import { SettlingTitle } from "./SettlingTitle";
import { Tooltip } from "./Tooltip/Tooltip";

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

/** Behind memo with no props: it subscribes to the store itself, so the
 * shell re-rendering per streamed frame never reaches it. */
export const ConversationTabs = memo(function ConversationTabs() {
  const {
    tabOrder,
    tabsById,
    activeTabId,
    companions,
    settlingTitles,
    setActiveTab,
    closeTab,
    openNewConversation,
  } = useCompanionStore(
    useShallow((state) => ({
      tabOrder: state.tabOrder,
      tabsById: state.tabsById,
      activeTabId: state.activeTabId,
      companions: state.companions,
      settlingTitles: state.settlingTitles,
      setActiveTab: state.setActiveTab,
      closeTab: state.closeTab,
      openNewConversation: state.openNewConversation,
    })),
  );
  const builtIn = companions.find((companion) => companion.isBuiltIn) ?? null;

  return (
    <div className="conversation-tabs" role="tablist" aria-label="Open conversations">
      <div className="conversation-tabs__track">
        {tabOrder.map((tabId) => {
          const tab = tabsById[tabId];
          if (!tab) return null;
          const isActive = tabId === activeTabId;
          // Each tab wears the face of who it talks to (s569). An unpicked
          // tab answers to the built-in companion, so it wears that face;
          // a companion without a picture wears the mark, as everywhere.
          const companion =
            companions.find((candidate) => candidate.id === tab.companionId) ?? builtIn;
          const settlingFrom = tab.conversationId
            ? (settlingTitles[tab.conversationId] ?? null)
            : null;
          return (
            <div
              className={`conversation-tab ${isActive ? "is-active" : ""} ${
                settlingFrom ? "is-settling" : ""
              }`}
              key={tabId}
            >
              <Tooltip content={tab.title} placement="bottom" whenTruncated>
                <button
                  className="conversation-tab__select"
                  type="button"
                  role="tab"
                  aria-selected={isActive}
                  onClick={() => setActiveTab(tabId)}
                >
                  <span className="conversation-tab__face">
                    <CompanionMark src={companion?.avatarUrl} />
                  </span>
                  {tab.unreadCount > 0 ? <span className="conversation-tab__unread" /> : null}
                  <SettlingTitle title={tab.title} from={settlingFrom} />
                </button>
              </Tooltip>
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
});
