import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { CallThread } from "./types";

/** Rust fires this when a call moved on its own — a woken companion answered
 *  one while nobody was looking at it. */
const CALLS_CHANGED_EVENT = "calls://changed";

/** Every call born out of one conversation, newest first, with its turns.
 *
 *  The only door this module opens onto Rust. Read-only and
 *  conversation-scoped — there is no command for "all calls", deliberately. */
export function listConversationCalls(conversationId: string): Promise<CallThread[]> {
  return invoke<CallThread[]>("list_conversation_calls", { conversationId });
}

/** Fires when a call changed without this window doing anything — the waker
 *  gave a companion a turn and it answered. Carries no payload on purpose: the
 *  only honest response is to re-read, and a diff would be a second source of
 *  truth about state this module already knows how to fetch. */
export function onCallsChanged(handler: () => void): Promise<UnlistenFn> {
  return listen(CALLS_CHANGED_EVENT, () => handler());
}
