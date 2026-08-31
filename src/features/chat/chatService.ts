import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  AcceptedMessage,
  ChatEvent,
  Conversation,
  ConversationThread,
  SubmitMessageInput,
  UpdateConversationCompanionInput,
} from "./types";

/** The app-wide bus for turns nobody asked for. The human lane rides its own
 *  per-submission channel; what arrives HERE is exclusively the woken lane —
 *  call answers, call close reports — plus the transient call speech the call
 *  cards own. */
const CHAT_EVENT = "chat://event";

export function listConversations(): Promise<Conversation[]> {
  return invoke<Conversation[]>("list_conversations");
}

export function getConversationThread(conversationId: string): Promise<ConversationThread> {
  return invoke<ConversationThread>("get_conversation_thread", { conversationId });
}

export function updateConversationCompanion(
  input: UpdateConversationCompanionInput,
): Promise<Conversation> {
  return invoke<Conversation>("update_conversation_companion", { input });
}

export function submitMessage(
  input: SubmitMessageInput,
  handleEvent: (event: ChatEvent) => void,
): Promise<AcceptedMessage> {
  const onEvent = new Channel<ChatEvent>();
  onEvent.onmessage = handleEvent;
  return invoke<AcceptedMessage>("submit_message", { input, onEvent });
}

/** Woken-lane chat events, app-wide. A window showing a thread a companion
 *  was woken in folds these to see the turn happen live instead of on the
 *  next reload. */
export function onWokenChatEvent(handler: (event: ChatEvent) => void): Promise<UnlistenFn> {
  return listen<ChatEvent>(CHAT_EVENT, ({ payload }) => handler(payload));
}
