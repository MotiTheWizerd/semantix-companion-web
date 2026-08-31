import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { CallSpeechEvent, ChatEvent } from "../chat/types";
import type { CallThread } from "./types";

/** Rust fires this when a call moved on its own — a woken companion answered
 *  one while nobody was looking at it. */
const CALLS_CHANGED_EVENT = "calls://changed";
/** Rust fires this the instant the waker starts a woken turn for a call —
 *  the reply is in flight from this moment. Every wake is followed by
 *  CALLS_CHANGED_EVENT when the attempt ends, success or failure, so an
 *  indicator armed on wake and disarmed on changed can never go stale. */
const CALL_WAKE_EVENT = "calls://wake";
const CHAT_EVENT = "chat://event";

/** The waker's payload: which call is being answered, and by whom. */
export interface CallWakeEvent {
  callId: string;
  agentId: string;
}

/** Every call born out of one conversation, newest first, with its turns.
 *
 *  The only door this module opens onto Rust. Read-only and
 *  conversation-scoped — there is no command for "all calls", deliberately. */
export function listConversationCalls(conversationId: string): Promise<CallThread[]> {
  return invoke<CallThread[]>("list_conversation_calls", { conversationId });
}

/** Ring again: clear the wake guard on an open call so the waker re-rings its
 *  newest turn within a tick. The human's door out of a stable silence — one
 *  press buys exactly one more wake. Resolves to whether anything re-armed;
 *  false means the call was already over or already ringing. */
export function retryCallWake(callId: string): Promise<boolean> {
  return invoke<boolean>("retry_call_wake", { callId });
}

/** Fires when a call changed without this window doing anything — the waker
 *  gave a companion a turn and it answered. Carries no payload on purpose: the
 *  only honest response is to re-read, and a diff would be a second source of
 *  truth about state this module already knows how to fetch. */
export function onCallsChanged(handler: () => void): Promise<UnlistenFn> {
  return listen(CALLS_CHANGED_EVENT, () => handler());
}

/** Fires when a woken turn starts answering a call. */
export function onCallWake(handler: (event: CallWakeEvent) => void): Promise<UnlistenFn> {
  return listen<CallWakeEvent>(CALL_WAKE_EVENT, ({ payload }) => handler(payload));
}

/** Provider-neutral live call speech. Other app-wide chat events share this
 * bus, so the filter is intentionally owned at this boundary. */
export function onCallSpeech(handler: (event: CallSpeechEvent) => void): Promise<UnlistenFn> {
  return listen<ChatEvent>(CHAT_EVENT, ({ payload }) => {
    if (payload.kind === "callSpeechDelta" || payload.kind === "callSpeechFinished") {
      handler(payload);
    }
  });
}
