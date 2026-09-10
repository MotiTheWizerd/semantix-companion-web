// ☎ CALL TRANSCRIPT ITEM — one exchange your companion had with another agent
// while working in this thread.
//
// A call is a first-class row in the conversation timeline — and a long one
// is several rows: the host cuts a call into segments, one per stretch
// between the conversation's own messages, and decides where each belongs.
// This component only knows how one segment looks: the opening wears the
// header, a continuation wears a slim strip, and the latest carries whatever
// is live. Open/closed is one state for the whole call, held by the host, so
// every segment of a call opens and closes together.

import { useEffect, useState } from "react";

import { retryCallWake } from "./callService";
import {
  type CallSegment,
  type RavenCallMessage,
  type StreamingCallMessage,
} from "./types";

/** How long a woken companion may compose before its quiet is called
 * "No answer". The wake guard lands BEFORE the woken turn runs (Rust's
 * anti-retry-storm law), so guard-on-newest-turn alone cannot separate
 * "thinking" from "gave up" — the wake stamp plus this window can. Generous
 * on purpose: a slow model deep in tool rounds is still answering. */
const ANSWER_BUDGET_MS = 120_000;

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

/** Who is speaking, as a face when the companion has one and its initial when
 *  it does not. Every turn in the card — spoken, typing, ringing — goes
 *  through here, so a companion never wears its picture on some of its own
 *  lines and a letter on the others. */
function SpeakerAvatar({
  agentId,
  name,
  agentAvatars,
}: {
  agentId: string;
  name: string;
  agentAvatars?: ReadonlyMap<string, string>;
}) {
  const avatarUrl = agentAvatars?.get(agentId);
  return (
    <span
      className={`calls__avatar${avatarUrl ? " calls__avatar--image" : ""}`}
      aria-hidden="true"
    >
      {avatarUrl ? (
        <img src={avatarUrl} alt="" draggable={false} />
      ) : (
        agentInitial(name)
      )}
    </span>
  );
}

interface CallTurnProps {
  message: Pick<RavenCallMessage, "fromAgentId" | "body"> &
    Partial<Pick<RavenCallMessage, "createdAt">>;
  initiatorAgentId: string;
  agentNames: ReadonlyMap<string, string>;
  /** Companion id → its picture, for the companions that have one. */
  agentAvatars?: ReadonlyMap<string, string>;
  streaming?: boolean;
}

function CallTurn({
  message,
  initiatorAgentId,
  agentNames,
  agentAvatars,
  streaming = false,
}: CallTurnProps) {
  const name = agentLabel(message.fromAgentId, agentNames);
  const side = message.fromAgentId === initiatorAgentId ? "initiator" : "recipient";

  return (
    <div className={`calls__turn calls__turn--${side}${streaming ? " calls__turn--streaming" : ""}`}>
      <SpeakerAvatar
        agentId={message.fromAgentId}
        name={name}
        agentAvatars={agentAvatars}
      />
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
  agentAvatars,
}: {
  agentId: string;
  initiatorAgentId: string;
  agentNames: ReadonlyMap<string, string>;
  agentAvatars?: ReadonlyMap<string, string>;
}) {
  const name = agentLabel(agentId, agentNames);
  const side = agentId === initiatorAgentId ? "initiator" : "recipient";
  // The reply began when this ghost appeared — the wake event has no
  // timestamp, and the mount is at most a render behind it.
  const [since] = useState(() => Date.now());
  const now = useNow(true);

  return (
    <div className={`calls__turn calls__turn--${side} calls__turn--ghost`}>
      <SpeakerAvatar agentId={agentId} name={name} agentAvatars={agentAvatars} />
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
  agentAvatars,
  now,
}: {
  message: RavenCallMessage;
  initiatorAgentId: string;
  agentNames: ReadonlyMap<string, string>;
  agentAvatars?: ReadonlyMap<string, string>;
  now: number;
}) {
  const name = agentLabel(message.toAgentId, agentNames);
  const side = message.toAgentId === initiatorAgentId ? "initiator" : "recipient";

  return (
    <div className={`calls__turn calls__turn--${side} calls__turn--ghost calls__turn--ringing`}>
      <SpeakerAvatar
        agentId={message.toAgentId}
        name={name}
        agentAvatars={agentAvatars}
      />
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
  reason,
  redialing,
  onRedial,
}: {
  agentId: string;
  agentNames: ReadonlyMap<string, string>;
  callId: string;
  /** Rust's account of why the wake produced nothing, when it has one. A
   *  named failure reads differently from a companion that simply did not
   *  reply — and it is known the moment it happens, not two minutes later. */
  reason: string | null;
  redialing: boolean;
  onRedial: (callId: string) => void;
}) {
  const name = agentLabel(agentId, agentNames);
  return (
    <div className="calls__silence" role="status">
      <p className="calls__silence-word">
        {reason
          ? `No answer — ${name}'s turn failed: ${reason}`
          : `No answer — ${name} was woken and no reply came.`}
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
  segment,
  expanded,
  onExpandedChange,
  streamingMessages = [],
  replyingAgentId = null,
  agentNames,
  agentAvatars,
}: {
  segment: CallSegment;
  /** Whether the call's turns are shown — the same answer for every segment
   *  of one call. */
  expanded: boolean;
  onExpandedChange: (callId: string, expanded: boolean) => void;
  streamingMessages?: StreamingCallMessage[];
  /** Who is composing a reply to this call right now — the waker's word, not
   *  a guess. Null when nothing is in flight. */
  replyingAgentId?: string | null;
  agentNames: ReadonlyMap<string, string>;
  /** Companion id → its picture, for the companions that have one. */
  agentAvatars?: ReadonlyMap<string, string>;
}) {
  const [redialing, setRedialing] = useState(false);
  const { thread, messages: turns, isOpening, isLatest } = segment;
  const { call, messages } = thread;
  const setExpanded = (next: boolean) => onExpandedChange(call.id, next);
  const used = call.messageCount;
  const other =
    messages.find((message) => message.fromAgentId !== call.initiatorAgentId)?.fromAgentId ??
    messages[0]?.toAgentId ??
    null;
  const initiatorName = agentLabel(call.initiatorAgentId, agentNames);
  const otherName = other ? agentLabel(other, agentNames) : null;
  const title = otherName ? `${initiatorName} called ${otherName}` : `${initiatorName} opened a call`;

  // The call's own clock: ticking while it is open, final once it closed.
  // Duration is a fact about the call, not about any phase, so it never hides.
  // The same tick re-derives the phases below, which is what lets "Replying"
  // honestly expire into "No answer" when the answering window runs out.
  const now = useNow(call.status === "open");
  const duration = formatElapsed((call.closedAt ?? now) - call.createdAt);

  // The card's phases, most specific first. "Speaking" is words actually
  // streaming; "replying" is the woken turn running before (or between) words —
  // hidden again once the reply has landed as the newest turn, because a woken
  // turn can outlive its own answer by a closing thought. Below those two,
  // Rust's wake guard splits the remaining quiet of an open call in half:
  // guard behind the newest turn means the phone is still RINGING for whoever
  // it addresses; guard on the newest turn means they were WOKEN — and the
  // wake stamp splits THAT in half again: a fresh wake is a companion
  // composing its answer ("Replying"), only a stale one is silence a person
  // may answer with "ring again". Before the stamp existed, the card called a
  // model mid-thought "No answer" the moment the waker picked up (seen live,
  // s533) — the guard lands BEFORE the turn runs, by design.
  const speaking = streamingMessages.length > 0;
  const newestMessage = messages.length > 0 ? messages[messages.length - 1] : null;
  const replying =
    replyingAgentId !== null &&
    call.status === "open" &&
    !speaking &&
    newestMessage?.fromAgentId !== replyingAgentId;
  const atRest = call.status === "open" && !speaking && !replying && newestMessage !== null;
  const ringing = atRest && call.wokenForMessageId !== newestMessage.id;
  // A wake Rust already knows failed is not "answering", however fresh its
  // stamp: the model died at the provider, and the window would only make
  // the card lie for two minutes before telling the truth (seen live, s572).
  const answering =
    atRest &&
    call.wokenForMessageId === newestMessage.id &&
    call.wakeError === null &&
    call.wokenAt !== null &&
    now - call.wokenAt < ANSWER_BUDGET_MS;
  const unanswered =
    atRest && call.wokenForMessageId === newestMessage.id && !answering;
  const liveWord =
    speaking ? "Speaking" : replying || answering ? "Replying" : ringing ? "Ringing" : null;
  const statusWord =
    liveWord ?? (call.status === "open" ? (unanswered ? "No answer" : "Open") : "Ended");

  // Anything happening live inside a collapsed call must be visible without
  // making the user notice a changing meter and manually open it mid-sentence.
  // A ring counts: a phone that rings where nobody can see it rings for nobody.
  // The latest segment is the one that shows the live part, so it is the one
  // that asks — once per call, not once per segment.
  const live = streamingMessages.length > 0 || replying || answering || ringing;
  useEffect(() => {
    if (isLatest && live) onExpandedChange(call.id, true);
  }, [isLatest, live, call.id, onExpandedChange]);

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

  const statusPill = (
    <span
      className={`calls__status calls__status--${call.status}${
        liveWord ? " calls__status--live" : ""
      }${unanswered ? " calls__status--silent" : ""}`}
    >
      <span aria-hidden="true" />
      {statusWord}
    </span>
  );

  // What this stretch shows when open: its own turns, and — on the latest
  // stretch only — the live tail. A call with nothing said yet says so once,
  // at its live end; an opening whose first line landed later stays quiet
  // rather than announcing an emptiness that is not true of the call.
  const nothingSaid = messages.length === 0 && streamingMessages.length === 0 && !replying;
  const tail = isLatest ? (
    <>
      {streamingMessages.map((message) => (
        <CallTurn
          key={message.streamId}
          message={message}
          initiatorAgentId={call.initiatorAgentId}
          agentNames={agentNames}
          agentAvatars={agentAvatars}
          streaming
        />
      ))}
      {replying && replyingAgentId !== null && (
        <TypingTurn
          agentId={replyingAgentId}
          initiatorAgentId={call.initiatorAgentId}
          agentNames={agentNames}
          agentAvatars={agentAvatars}
        />
      )}
      {ringing && newestMessage !== null && (
        <RingingTurn
          message={newestMessage}
          initiatorAgentId={call.initiatorAgentId}
          agentNames={agentNames}
          agentAvatars={agentAvatars}
          now={now}
        />
      )}
      {unanswered && newestMessage !== null && (
        <SilenceNotice
          agentId={newestMessage.toAgentId}
          agentNames={agentNames}
          callId={call.id}
          reason={call.wakeError}
          redialing={redialing}
          onRedial={redial}
        />
      )}
    </>
  ) : null;
  // An opening with no turns of its own and nothing live is header only.
  const hasBody = turns.length > 0 || isLatest;
  const body =
    expanded && hasBody ? (
      <div className="calls__turns">
        {isLatest && nothingSaid ? (
          <p className="calls__empty">Nothing was said in this call.</p>
        ) : (
          <>
            {turns.map((message) => (
              <CallTurn
                key={message.id}
                message={message}
                initiatorAgentId={call.initiatorAgentId}
                agentNames={agentNames}
                agentAvatars={agentAvatars}
              />
            ))}
            {tail}
          </>
        )}
      </div>
    ) : null;

  if (!isOpening) {
    // A continuation: the call carried on after the companion had said more
    // in the thread. A slim strip names it and toggles the same open state
    // as the header above; the status rides here only while this is the
    // live end of the call, so "Ended" reads where the call actually ended.
    return (
      <article className="chat-message chat-message--call">
        <div
          className={`calls calls--${call.status} calls--continued`}
          role="group"
          aria-label={`Companion call, continued: ${title}`}
        >
          <div className="calls__row">
            <button
              type="button"
              className="calls__continued"
              aria-expanded={expanded}
              onClick={() => setExpanded(!expanded)}
            >
              <span className="calls__continued-icon">
                <PhoneIcon />
              </span>
              <span className="calls__continued-title">{title} · continued</span>
              {isLatest ? statusPill : null}
              <span className={`calls__chevron${expanded ? " calls__chevron--open" : ""}`}>
                <ChevronIcon />
              </span>
            </button>
            {body}
          </div>
        </div>
      </article>
    );
  }

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
            onClick={() => setExpanded(!expanded)}
          >
            <span className="calls__icon">
              <PhoneIcon />
            </span>
            <span className="calls__heading">
              <span className="calls__eyebrow">Companion call</span>
              <span className="calls__title">{title}</span>
            </span>
            <span className="calls__summary-meta">
              {statusPill}
              <span
                className="calls__duration"
                title={call.status === "open" ? "Call running for" : "Call lasted"}
              >
                {duration}
              </span>
              <span className="calls__meter">
                <strong>{used}</strong> / {call.messageLimit} turns
              </span>
            </span>
            <span className={`calls__chevron${expanded ? " calls__chevron--open" : ""}`}>
              <ChevronIcon />
            </span>
          </button>
          {body}
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
