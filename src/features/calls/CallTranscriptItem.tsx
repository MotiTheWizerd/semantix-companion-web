// ☎ CALL TRANSCRIPT ITEM — one exchange your companion had with another agent
// while working in this thread.
//
// A call is a first-class row in the conversation timeline. The host decides
// where it belongs; this component only knows how one call looks and expands.

import { useState } from "react";

import { MAX_MESSAGES_PER_CALL, type CallThread } from "./types";

function shortId(id: string): string {
  return id.slice(0, 8);
}

function speakerLabel(agentId: string, initiatorAgentId: string): string {
  return agentId === initiatorAgentId ? "your companion" : `agent ${shortId(agentId)}`;
}

export function CallTranscriptItem({ thread }: { thread: CallThread }) {
  const [expanded, setExpanded] = useState(false);
  const { call, messages } = thread;
  const used = call.messageCount;
  const other =
    messages.find((message) => message.fromAgentId !== call.initiatorAgentId)?.fromAgentId ??
    messages[0]?.toAgentId ??
    null;

  return (
    <article className="chat-message chat-message--call">
      <div className="calls" role="group" aria-label="Companion call">
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
      </div>
    </article>
  );
}

export function CallTranscriptError({ error }: { error: string }) {
  return (
    <article className="chat-message chat-message--call">
      <div className="calls" role="status">
        <p className="calls__error">☎ Could not read this conversation's calls — {error}</p>
      </div>
    </article>
  );
}
