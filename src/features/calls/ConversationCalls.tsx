// ☎ CONVERSATION CALLS — the exchanges your companion had with other agents
// while working in this thread.
//
// ⚑ DELIBERATELY SELF-CONTAINED. It takes two things its host already knows —
// which conversation, and whether a turn is running — and reaches for nothing
// else. No chat store, no message list, no shared state. Its host renders one
// element and learns nothing about calls; moving this into a sidebar, a
// settings pane or its own window is moving one line.
//
// It renders NOTHING when there are no calls, so a thread that never placed
// one looks exactly as it did before this existed.

import { useState } from "react";

import { useConversationCalls } from "./useConversationCalls";
import { MAX_MESSAGES_PER_CALL, type CallThread } from "./types";

interface ConversationCallsProps {
  conversationId: string | null;
  /** True while a turn is streaming. The falling edge is when calls are
   *  refetched — see useConversationCalls. */
  turnInProgress: boolean;
}

function shortId(id: string): string {
  return id.slice(0, 8);
}

function speakerLabel(agentId: string, initiatorAgentId: string): string {
  return agentId === initiatorAgentId ? "your companion" : `agent ${shortId(agentId)}`;
}

function CallRow({ thread }: { thread: CallThread }) {
  const [expanded, setExpanded] = useState(false);
  const { call, messages } = thread;
  const used = call.messageCount;
  const other =
    messages.find((message) => message.fromAgentId !== call.initiatorAgentId)?.fromAgentId ??
    messages[0]?.toAgentId ??
    null;

  return (
    <div className="calls__row">
      <button
        type="button"
        className="calls__summary"
        aria-expanded={expanded}
        onClick={() => setExpanded((open) => !open)}
      >
        <span aria-hidden="true">☎</span>
        <span className="calls__title">
          {other ? `called ${shortId(other)}` : "opened a call"}
        </span>
        <span className={`calls__status calls__status--${call.status}`}>{call.status}</span>
        <span className="calls__meter">
          {used}/{MAX_MESSAGES_PER_CALL}
        </span>
      </button>

      {expanded && (
        <div className="calls__turns">
          {messages.length === 0 ? (
            <p className="calls__empty">Nothing was said in this call.</p>
          ) : (
            messages.map((message) => (
              <div key={message.id} className="calls__turn">
                <span className="calls__speaker">
                  {speakerLabel(message.fromAgentId, call.initiatorAgentId)}
                </span>
                <p className="calls__body">{message.body}</p>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}

export function ConversationCalls({ conversationId, turnInProgress }: ConversationCallsProps) {
  const { threads, error } = useConversationCalls(conversationId, turnInProgress);

  if (error) {
    return (
      <div className="calls" role="status">
        <p className="calls__error">☎ Could not read this conversation's calls — {error}</p>
      </div>
    );
  }
  if (threads.length === 0) return null;

  return (
    <div className="calls" role="group" aria-label="Calls placed in this conversation">
      <p className="calls__heading">
        {threads.length === 1
          ? "1 call placed on your behalf"
          : `${threads.length} calls placed on your behalf`}
      </p>
      {threads.map((thread) => (
        <CallRow key={thread.call.id} thread={thread} />
      ))}
    </div>
  );
}
