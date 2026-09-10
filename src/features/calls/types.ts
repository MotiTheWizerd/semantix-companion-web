/** A call's lifecycle. It closes itself on its last message. */
export type CallStatus = "open" | "closed";

/** One exchange between two companions, born out of a conversation. */
export interface RavenCall {
  id: string;
  /** The conversation it came from. Null means unrooted — a call with no
   *  human thread behind it, which this surface never shows. */
  rootConversationId: string | null;
  initiatorAgentId: string;
  status: CallStatus;
  messageCount: number;
  /** How many turns the call may hold, from the same constant the repository
   *  enforces. The meter draws this; nothing on this side has a number of
   *  its own. */
  messageLimit: number;
  createdAt: number;
  closedAt: number | null;
  /** The newest turn a wake has fired for — Rust's wake guard, on the wire.
   *  Behind the newest message: the phone is still ringing. On the newest
   *  message with no reply after it: the other side was woken and stayed
   *  silent. Those are the two silences this field tells apart. */
  wokenForMessageId: string | null;
  /** WHEN that wake fired (ms). The guard lands before the woken turn runs,
   *  so this stamp is what separates "picked up, composing" from "gave up" —
   *  a fresh wake renders as Replying, only a stale one as No answer. */
  wokenAt: number | null;
  /** Why the wake for `wokenForMessageId` produced nothing, when Rust knows:
   *  a provider refusal, an unconfigured model, a turn that died mid-stream.
   *  Null while a wake is in flight or succeeded, and after any re-ring. With
   *  it the card names the failure at once instead of showing "Replying" for
   *  a model that already died and then an unexplained silence. */
  wakeError: string | null;
}

export interface RavenCallMessage {
  id: string;
  callId: string;
  fromAgentId: string;
  toAgentId: string;
  body: string;
  createdAt: number;
}

/** One not-yet-persisted call line while a provider is still producing the
 * `send_in_call` JSON arguments. SQLite replaces it after tool execution. */
export interface StreamingCallMessage {
  streamId: string;
  callId: string;
  fromAgentId: string;
  body: string;
}

/** A call and its turns, as Rust hands them over in one trip. */
export interface CallThread {
  call: RavenCall;
  messages: RavenCallMessage[];
}

/** One stretch of a call as it sits in the thread: the turns that happened
 *  between two of the conversation's own rows. A call that outlives the
 *  companion's next words is shown in pieces, each where it happened — the
 *  opening where the call was placed, the reply where it arrived — instead
 *  of one block that pulls later turns above text written before them. */
export interface CallSegment {
  thread: CallThread;
  /** This stretch's turns, oldest first. May be empty for an opening whose
   *  first line landed after the companion had already said more. */
  messages: RavenCallMessage[];
  /** The first stretch: it carries the header — who called whom, the meter,
   *  the clock. */
  isOpening: boolean;
  /** The newest stretch: it carries whatever is live — words streaming, a
   *  reply in flight, the ring, the silence — and the call's close. */
  isLatest: boolean;
}
