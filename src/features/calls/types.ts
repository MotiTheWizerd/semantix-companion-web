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
  createdAt: number;
  closedAt: number | null;
  /** The newest turn a wake has fired for — Rust's wake guard, on the wire.
   *  Behind the newest message: the phone is still ringing. On the newest
   *  message with no reply after it: the other side was woken and stayed
   *  silent. Those are the two silences this field tells apart. */
  wokenForMessageId: string | null;
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

/** Mirrors MAX_MESSAGES_PER_CALL in Rust — display only. The limit is
 *  enforced in the repository; this is just how the meter is drawn. */
export const MAX_MESSAGES_PER_CALL = 5;
