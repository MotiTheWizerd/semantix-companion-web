import type { ModelPreference } from "../preferences/types";

export type MessageRole = "user" | "assistant" | "system" | "tool";
export type MessageStatus = "pending" | "streaming" | "completed" | "failed" | "cancelled";

export interface Conversation {
  id: string;
  title: string;
  modelPreference: ModelPreference;
  createdAt: number;
  updatedAt: number;
  archivedAt: number | null;
}

export interface ChatMessage {
  id: string;
  conversationId: string;
  sequence: number;
  role: MessageRole;
  status: MessageStatus;
  content: string;
  providerId: string | null;
  modelId: string | null;
  errorMessage: string | null;
  createdAt: number;
  updatedAt: number;
  completedAt: number | null;
}

export interface ConversationThread {
  conversation: Conversation;
  messages: ChatMessage[];
}

export interface AcceptedMessage {
  conversation: Conversation;
  message: ChatMessage;
}

export interface SubmitMessageInput {
  conversationId: string | null;
  modelPreference: ModelPreference;
  content: string;
  /** Recalled memory + time blocks — rides the inference request as a leading
   *  system message, never persisted into the conversation. */
  memoryContext?: string | null;
  /** The memory agent backing the recall_memory tool; null = tool undeclared. */
  memoryAgentId?: string | null;
}

/** What the 📖 chip renders for one tool call — latest lifecycle state. */
export interface ToolCallChipItem {
  callId: string;
  name: string;
  arguments: string;
  status: "running" | "ok" | "error";
  detail: string | null;
}

/** One tool call's lifecycle on an assistant message — instrument data,
 *  runtime-held like the 🧠 chip, never persisted. */
export interface ToolCallEvent {
  kind: "toolCall";
  conversationId: string;
  messageId: string;
  callId: string;
  name: string;
  arguments: string;
  status: "running" | "ok" | "error";
  detail: string | null;
}

export interface UpdateConversationModelPreferenceInput {
  conversationId: string;
  modelPreference: ModelPreference;
}

export type ChatEvent =
  | ({ kind: "accepted" } & AcceptedMessage)
  | { kind: "assistantStarted"; message: ChatMessage }
  | {
      kind: "assistantDelta";
      conversationId: string;
      messageId: string;
      sequence: number;
      delta: string;
    }
  | { kind: "assistantCompleted"; message: ChatMessage }
  | {
      kind: "failed";
      conversationId: string;
      messageId: string | null;
      message: string;
    }
  | ToolCallEvent;
