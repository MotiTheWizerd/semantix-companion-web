import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  AcceptedMessage,
  ChatEvent,
  Conversation,
  ConversationThread,
  SubmitMessageInput,
  UpdateConversationCompanionInput,
} from "./types";

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
