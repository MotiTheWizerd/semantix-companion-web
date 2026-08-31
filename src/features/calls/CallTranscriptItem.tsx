// ☎ CALL TRANSCRIPT ITEM — one exchange your companion had with another agent
// while working in this thread.
//
// A call is a first-class row in the conversation timeline. The host decides
// where it belongs; this component only knows how one call looks and expands.

import { useEffect, useState } from "react";

import { retryCallWake } from "./callService";
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

/** "0:07", "4:12", "1:04:07" — a phone's clock, not a log's. */
function formatElapsed(milliseconds: number): string {
  const total = Math.max(0, Math.floor(milliseconds / 1000));
  const seconds = total % 60;
  const minutes = Math.floor(total / 60) % 60;
  const hours = Math.floor(total / 3600);
  const padded = String(seconds).padStart(2, "0");
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${padded}`
    : `${minutes}:${padded}`;
}

/** The current time, re-read every second while `active` — the one clock all
 *  of a card's tickers share, so they advance together instead of drifting. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active]);
  return now;
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

/** The messenger typing bubble, for a reply the waker told us is in flight.
 *  A ghost turn, not a message — it holds the newest slot until words start
 *  streaming (the draft replaces it) or the woken turn ends. */
function TypingTurn({
  agentId,
  initiatorAgentId,
  agentNames,
}: {
  agentId: string;
  initiatorAgentId: string;
  agentNames: ReadonlyMap<string, string>;
}) {
  const name = agentLabel(agentId, agentNames);
  const side = agentId === initiatorAgentId ? "initiator" : "recipient";
  // The reply began when this ghost appeared — the wake event has no
  // timestamp, and the mount is at most a render behind it.
  const [since] = useState(() => Date.now());
  const now = useNow(true);

  return (
    <div className={`calls__turn calls__turn--${side} calls__turn--ghost`}>
      <span className="calls__avatar" aria-hidden="true">
        {agentInitial(name)}
      </span>
      <div className="calls__turn-content">
        <div className="calls__turn-meta">
          <span className="calls__speaker">{name}</span>
          <span className="calls__live">
            <span aria-hidden="true" /> Replying · {formatElapsed(now - since)}
          </span>
        </div>
        <span className="calls__typing" aria-hidden="true">
          <span />
          <span />
          <span />
        </span>
      </div>
    </div>
  );
}

/** The phone, mid-ring. The newest turn is addressed to someone the waker has
 *  not reached yet — its own timestamp is when the ringing began, so this
 *  clock survives a window reopen where a session timer would reset. */
function RingingTurn({
  message,
  initiatorAgentId,
  agentNames,
  now,
}: {
  message: RavenCallMessage;
  initiatorAgentId: string;
  agentNames: ReadonlyMap<string, string>;
  now: number;
}) {
  const name = agentLabel(message.toAgentId, agentNames);
  const side = message.toAgentId === initiatorAgentId ? "initiator" : "recipient";

  return (
    <div className={`calls__turn calls__turn--${side} calls__turn--ghost calls__turn--ringing`}>
      <span className="calls__avatar" aria-hidden="true">
        {agentInitial(name)}
      </span>
      <div className="calls__turn-content">
        <div className="calls__turn-meta">
          <span className="calls__speaker">{name}</span>
          <span className="calls__live calls__live--ringing">
            <span aria-hidden="true" /> Ringing · {formatElapsed(now - message.createdAt)}
          </span>
        </div>
        <span className="calls__ring-pulse" aria-hidden="true">
          <span />
          <span />
          <span />
        </span>
      </div>
    </div>
  );
}

/** The wake fired for the newest turn and nothing came back — a decline, a
 *  dead model, or a companion that read and moved on. From the outside those
 *  are one fact: no answer. The button clears the wake guard so the waker
 *  rings once more; each press buys exactly one retry, never a loop. */
function SilenceNotice({
  agentId,
  agentNames,
  callId,
  redialing,
  onRedial,
}: {
  agentId: string;
  agentNames: ReadonlyMap<string, string>;
  callId: string;
  redialing: boolean;
  onRedial: (callId: string) => void;
}) {
  const name = agentLabel(agentId, agentNames);
  return (
    <div className="calls__silence" role="status">
      <p className="calls__silence-word">
        No answer — {name} was woken and no reply came.
      </p>
      <button
        type="button"
        className="calls__redial"
        disabled={redialing}
        onClick={() => onRedial(callId)}
      >
        {redialing ? "Ringing…" : "Ring again"}
      </button>
    </div>
  );
}

export function CallTranscriptItem({
  thread,
  streamingMessages = [],
  replyingAgentId = null,
  agentNames,
}: {
  thread: CallThread;
  streamingMessages?: StreamingCallMessage[];
  /** Who is composing a reply to this call right now — the waker's word, not
   *  a guess. Null when nothing is in flight. */
  replyingAgentId?: string | null;
  agentNames: ReadonlyMap<string, string>;
}) {
  const [expanded, setExpanded] = useState(false);
  const [redialing, setRedialing] = useState(false);
  const { call, messages } = thread;
  const used = call.messageCount;
  const other =
    messages.find((message) => message.fromAgentId !== call.initiatorAgentId)?.fromAgentId ??
    messages[0]?.toAgentId ??
    null;
  const initiatorName = agentLabel(call.initiatorAgentId, agentNames);
  const otherName = other ? agentLabel(other, agentNames) : null;
  const title = otherName ? `${initiatorName} called ${otherName}` : `${initiatorName} opened a call`;

  // The card's phases, most specific first. "Speaking" is words actually
  // streaming; "replying" is the woken turn running before (or between) words —
  // hidden again once the reply has landed as the newest turn, because a woken
  // turn can outlive its own answer by a closing thought. Below those two,
  // Rust's wake guard splits the remaining quiet of an open call in half:
  // guard behind the newest turn means the phone is still RINGING for whoever
  // it addresses; guard on the newest turn means they were woken and stayed
  // SILENT — the state a person may answer with "ring again".
  const speaking = streamingMessages.length > 0;
  const newestMessage = messages.length > 0 ? messages[messages.length - 1] : null;
  const replying =
    replyingAgentId !== null &&
    call.status === "open" &&
    !speaking &&
    newestMessage?.fromAgentId !== replyingAgentId;
  const atRest = call.status === "open" && !speaking && !replying && newestMessage !== null;
  const ringing = atRest && call.wokenForMessageId !== newestMessage.id;
  const unanswered = atRest && call.wokenForMessageId === newestMessage.id;
  const liveWord = speaking ? "Speaking" : replying ? "Replying" : ringing ? "Ringing" : null;
  const statusWord =
    liveWord ?? (call.status === "open" ? (unanswered ? "No answer" : "Open") : "Ended");

  // The call's own clock: ticking while it is open, final once it closed.
  // Duration is a fact about the call, not about any phase, so it never hides.
  const now = useNow(call.status === "open");
  const duration = formatElapsed((call.closedAt ?? now) - call.createdAt);

  // Anything happening live inside a collapsed call must be visible without
  // making the user notice a changing meter and manually open it mid-sentence.
  // A ring counts: a phone that rings where nobody can see it rings for nobody.
  useEffect(() => {
    if (streamingMessages.length > 0 || replying || ringing) setExpanded(true);
  }, [streamingMessages.length, replying, ringing]);

  // One press, one retry. Success flips the card back to ringing through the
  // refetch Rust's changed event triggers; the effect below re-arms the button
  // only when the silence state has genuinely been left and re-entered.
  useEffect(() => {
    if (!unanswered) setRedialing(false);
  }, [unanswered]);
  const redial = (callId: string) => {
    setRedialing(true);
    retryCallWake(callId)
      .then((rearmed) => {
        // False means the call closed under us — nothing will refetch, so
        // the button must not stay dead in a state that will not change.
        if (!rearmed) setRedialing(false);
      })
      .catch(() => setRedialing(false));
  };

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
              <span
                className={`calls__status calls__status--${call.status}${
                  liveWord ? " calls__status--live" : ""
                }${unanswered ? " calls__status--silent" : ""}`}
              >
                <span aria-hidden="true" />
                {statusWord}
              </span>
              <span
                className="calls__duration"
                title={call.status === "open" ? "Call running for" : "Call lasted"}
              >
                {duration}
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
              {messages.length === 0 && streamingMessages.length === 0 && !replying ? (
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
                  {replying && replyingAgentId !== null && (
                    <TypingTurn
                      agentId={replyingAgentId}
                      initiatorAgentId={call.initiatorAgentId}
                      agentNames={agentNames}
                    />
                  )}
                  {ringing && newestMessage !== null && (
                    <RingingTurn
                      message={newestMessage}
                      initiatorAgentId={call.initiatorAgentId}
                      agentNames={agentNames}
                      now={now}
                    />
                  )}
                  {unanswered && newestMessage !== null && (
                    <SilenceNotice
                      agentId={newestMessage.toAgentId}
                      agentNames={agentNames}
                      callId={call.id}
                      redialing={redialing}
                      onRedial={redial}
                    />
                  )}
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
