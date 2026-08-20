export type MessageRole = "user" | "assistant" | "system" | "tool";
export type MessageStatus = "pending" | "streaming" | "completed" | "failed" | "cancelled";

export interface Conversation {
  id: string;
  title: string;
  selectedModelId: string | null;
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
  content: string;
}

export type ChatEvent =
  | ({ kind: "accepted" } & AcceptedMessage)
  | { kind: "assistantStarted"; message: ChatMessage }
  | { kind: "assistantDelta"; conversationId: string; messageId: string; delta: string }
  | { kind: "assistantCompleted"; message: ChatMessage }
  | {
      kind: "failed";
      conversationId: string;
      messageId: string | null;
      message: string;
    };
