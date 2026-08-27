// ☎ CALL TRANSCRIPT ITEM — one exchange your companion had with another agent
// while working in this thread.
//
// A call is a first-class row in the conversation timeline. The host decides
// where it belongs; this component only knows how one call looks and expands.

import { useEffect, useState } from "react";

import {
  MAX_MESSAGES_PER_CALL,
  type CallThread,
  type RavenCallMessage,
  type StreamingCallMessage,
} from "./types";

function shortId(id: string): string {
  return id.slice(0, 8);
}

function agentLabel(agentId: string, agentNames: ReadonlyMap<string, string>): string {
  return agentNames.get(agentId) ?? `Agent ${shortId(agentId)}`;
}

function agentInitial(label: string): string {
  return label.trim().charAt(0).toUpperCase() || "•";
}

function PhoneIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="M5.1 3.9c.5-.5 1.3-.5 1.8 0l1.5 1.5c.4.4.5 1 .2 1.5l-.8 1.3a.8.8 0 0 0 .1.9l3 3a.8.8 0 0 0 .9.1l1.3-.8c.5-.3 1.1-.2 1.5.2l1.5 1.5c.5.5.5 1.3 0 1.8l-.8.8c-.9.9-2.2 1.2-3.4.7a15 15 0 0 1-8.3-8.3c-.5-1.2-.2-2.5.7-3.4l.8-.8Z" />
    </svg>
  );
}

function ChevronIcon() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="m4 6 4 4 4-4" />
    </svg>
  );
}

const CALL_TIME_FORMATTER = new Intl.DateTimeFormat(undefined, {
  hour: "numeric",
  minute: "2-digit",
});

function messageTime(timestamp: number): string {
  return CALL_TIME_FORMATTER.format(timestamp);
}

interface CallTurnProps {
  message: Pick<RavenCallMessage, "fromAgentId" | "body"> &
    Partial<Pick<RavenCallMessage, "createdAt">>;
  initiatorAgentId: string;
  agentNames: ReadonlyMap<string, string>;
  streaming?: boolean;
}

function CallTurn({
  message,
  initiatorAgentId,
  agentNames,
  streaming = false,
}: CallTurnProps) {
  const name = agentLabel(message.fromAgentId, agentNames);
  const side = message.fromAgentId === initiatorAgentId ? "initiator" : "recipient";

  return (
    <div className={`calls__turn calls__turn--${side}${streaming ? " calls__turn--streaming" : ""}`}>
      <span className="calls__avatar" aria-hidden="true">
        {agentInitial(name)}
      </span>
      <div className="calls__turn-content">
        <div className="calls__turn-meta">
          <span className="calls__speaker">{name}</span>
          {streaming ? (
            <span className="calls__live">
              <span aria-hidden="true" /> Speaking
            </span>
          ) : message.createdAt !== undefined ? (
            <time dateTime={new Date(message.createdAt).toISOString()}>
              {messageTime(message.createdAt)}
            </time>
          ) : null}
        </div>
        <p className={`calls__body${streaming ? " calls__body--streaming" : ""}`}>
          {message.body}
        </p>
      </div>
    </div>
  );
}

export function CallTranscriptItem({
  thread,
  streamingMessages = [],
  agentNames,
}: {
  thread: CallThread;
  streamingMessages?: StreamingCallMessage[];
  agentNames: ReadonlyMap<string, string>;
}) {
  const [expanded, setExpanded] = useState(false);
  const { call, messages } = thread;
  const used = call.messageCount;
  const other =
    messages.find((message) => message.fromAgentId !== call.initiatorAgentId)?.fromAgentId ??
    messages[0]?.toAgentId ??
    null;
  const initiatorName = agentLabel(call.initiatorAgentId, agentNames);
  const otherName = other ? agentLabel(other, agentNames) : null;
  const title = otherName ? `${initiatorName} called ${otherName}` : `${initiatorName} opened a call`;

  // Speech arriving into a collapsed call must be visible without making the
  // user notice a changing meter and manually open it mid-sentence.
  useEffect(() => {
    if (streamingMessages.length > 0) setExpanded(true);
  }, [streamingMessages.length]);

  return (
    <article className="chat-message chat-message--call">
      <div
        className={`calls calls--${call.status}`}
        role="group"
        aria-label={`Companion call: ${title}`}
      >
        <div className="calls__row">
          <button
            type="button"
            className="calls__summary"
            aria-expanded={expanded}
            onClick={() => setExpanded((open) => !open)}
          >
            <span className="calls__icon">
              <PhoneIcon />
            </span>
            <span className="calls__heading">
              <span className="calls__eyebrow">Companion call</span>
              <span className="calls__title">{title}</span>
            </span>
            <span className="calls__summary-meta">
              <span className={`calls__status calls__status--${call.status}`}>
                <span aria-hidden="true" />
                {call.status === "open" ? "Open" : "Ended"}
              </span>
              <span className="calls__meter">
                <strong>{used}</strong> / {MAX_MESSAGES_PER_CALL} turns
              </span>
            </span>
            <span className={`calls__chevron${expanded ? " calls__chevron--open" : ""}`}>
              <ChevronIcon />
            </span>
          </button>

          {expanded && (
            <div className="calls__turns">
              {messages.length === 0 && streamingMessages.length === 0 ? (
                <p className="calls__empty">Nothing was said in this call.</p>
              ) : (
                <>
                  {messages.map((message) => (
                    <CallTurn
                      key={message.id}
                      message={message}
                      initiatorAgentId={call.initiatorAgentId}
                      agentNames={agentNames}
                    />
                  ))}
                  {streamingMessages.map((message) => (
                    <CallTurn
                      key={message.streamId}
                      message={message}
                      initiatorAgentId={call.initiatorAgentId}
                      agentNames={agentNames}
                      streaming
                    />
                  ))}
                </>
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
