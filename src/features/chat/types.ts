export type MessageRole = "user" | "assistant" | "system" | "tool";
export type MessageStatus = "pending" | "streaming" | "completed" | "failed" | "cancelled";

export interface Conversation {
  id: string;
  title: string;
  /** Who this thread talks to. The companion carries the model and the memory,
   *  so the conversation itself holds neither. */
  companionId: string | null;
  createdAt: number;
  updatedAt: number;
  archivedAt: number | null;
}

/** One image stored with a message. `data` is base64, no data-URL prefix. */
export interface MessageAttachment {
  id: string;
  mediaType: string;
  data: string;
}

/** A composer image awaiting send — same payload, identity minted by Rust. */
export interface PendingAttachment {
  id: string;
  mediaType: string;
  data: string;
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
  attachments: MessageAttachment[];
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
  /** The companion picked in the composer; null falls back to the thread's
   *  stored companion, then to the built-in one. */
  companionId: string | null;
  content: string;
  /** Recalled memory + time blocks — rides the inference request as a leading
   *  system message, never persisted into the conversation. */
  memoryContext?: string | null;
  /** The memory agent backing the recall_memory tool; null = tool undeclared. */
  memoryAgentId?: string | null;
  /** Images riding with the message — already downscaled by the composer. */
  attachments?: { mediaType: string; data: string }[];
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

/** Transient call speech rides the app event bus. It is deliberately part of
 * the canonical chat event vocabulary even though the call feature, not the
 * conversation store, owns its UI state. */
export type CallSpeechEvent =
  | {
      kind: "callSpeechDelta";
      streamId: string;
      callId: string;
      fromAgentId: string;
      delta: string;
    }
  | {
      kind: "callSpeechFinished";
      callId: string;
      fromAgentId: string;
      body: string;
      succeeded: boolean;
    };

export interface UpdateConversationCompanionInput {
  conversationId: string;
  companionId: string;
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
  | ToolCallEvent
  | CallSpeechEvent;
