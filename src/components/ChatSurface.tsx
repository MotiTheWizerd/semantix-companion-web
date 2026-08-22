import { useEffect, useRef, type FormEvent, type KeyboardEvent } from "react";

import { onConversationScrollToEnd } from "../features/chat/chatScrollEvents";
import type { ChatMessage, ToolCallChipItem } from "../features/chat/types";
import type { ConfiguredModel } from "../features/models/configuredModels/types";
import {
  modelPreferenceFromValue,
  modelPreferenceValue,
  type ModelPreference,
  type UserPreferences,
} from "../features/preferences/types";
import type { MemoryRecallChipData } from "../features/memory";
import { effectiveModelPreference } from "../features/workspace/companionStore";
import { EmptyState } from "./EmptyState";
import { MemoryRecallChip } from "./MemoryRecallChip";
import { ToolCallChip } from "./ToolCallChip";

function AttachIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="m7.75 10.75 4.4-4.4a2.3 2.3 0 1 1 3.25 3.25l-5.85 5.85a3.45 3.45 0 0 1-4.88-4.88l6.1-6.1" />
    </svg>
  );
}

function SendIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="M10 15.5v-11M5.75 8.75 10 4.5l4.25 4.25" />
    </svg>
  );
}

interface ChatSurfaceProps {
  activeConversationId: string | null;
  messages: ChatMessage[];
  isLoading: boolean;
  isSending: boolean;
  error: string | null;
  notice: string | null;
  /** 🧠 chip data per sent user message — live-session only. */
  recallByMessageId: Record<string, MemoryRecallChipData>;
  /** 📖 tool chips per assistant message — live-session only. */
  toolCallsByMessageId: Record<string, ToolCallChipItem[]>;
  content: string;
  configuredModels: ConfiguredModel[];
  modelPreference: ModelPreference;
  userPreferences: UserPreferences;
  onContentChange: (content: string) => void;
  onModelPreferenceChange: (preference: ModelPreference) => void;
  onSend: (content: string) => Promise<void>;
}

export function ChatSurface({
  activeConversationId,
  messages,
  isLoading,
  isSending,
  error,
  notice,
  recallByMessageId,
  toolCallsByMessageId,
  content,
  configuredModels,
  modelPreference,
  userPreferences,
  onContentChange,
  onModelPreferenceChange,
  onSend,
}: ChatSurfaceProps) {
  const threadRef = useRef<HTMLElement>(null);
  const activeConversationIdRef = useRef(activeConversationId);
  activeConversationIdRef.current = activeConversationId;
  const hasMessages = messages.length > 0;
  const inheritedModel = effectiveModelPreference({ mode: "inherit" }, userPreferences);
  const inheritedModelLabel =
    inheritedModel.mode === "configured"
      ? (configuredModels.find((model) => model.id === inheritedModel.modelId)?.displayName ??
        "Unavailable model")
      : "Test stream";

  useEffect(() => {
    let frameId: number | null = null;
    let requestedConversationId: string | null = null;
    const stopListening = onConversationScrollToEnd((conversationId) => {
      requestedConversationId = conversationId;
      if (frameId !== null) cancelAnimationFrame(frameId);
      frameId = requestAnimationFrame(() => {
        frameId = null;
        if (requestedConversationId !== activeConversationIdRef.current) return;
        const thread = threadRef.current;
        if (thread) thread.scrollTop = thread.scrollHeight;
      });
    });

    return () => {
      if (frameId !== null) cancelAnimationFrame(frameId);
      stopListening();
    };
  }, []);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const message = content.trim();
    if (!message || isSending) return;
    await onSend(message);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      event.currentTarget.form?.requestSubmit();
    }
  };

  return (
    <main className={`chat-surface ${hasMessages ? "has-messages" : ""}`} id="chat">
      {hasMessages ? (
        <section
          className="chat-thread"
          ref={threadRef}
          aria-label="Conversation messages"
          aria-live="polite"
        >
          {messages.map((message) => {
            const recall = recallByMessageId[message.id];
            const toolCalls = toolCallsByMessageId[message.id];
            return (
              <article
                className={`chat-message chat-message--${message.role}`}
                key={message.id}
              >
                <p>{message.content || (message.status === "streaming" ? "Thinking…" : "")}</p>
                {message.errorMessage ? (
                  <span className="chat-message__error">{message.errorMessage}</span>
                ) : null}
                {recall ? <MemoryRecallChip data={recall} /> : null}
                {toolCalls ? <ToolCallChip calls={toolCalls} /> : null}
              </article>
            );
          })}
        </section>
      ) : (
        <EmptyState />
      )}

      <form className="chat-composer" onSubmit={handleSubmit}>
        <label className="sr-only" htmlFor="companion-message">
          Message Companion
        </label>
        <textarea
          id="companion-message"
          name="message"
          rows={1}
          placeholder="Message Companion…"
          aria-describedby="composer-note"
          value={content}
          disabled={isLoading}
          onChange={(event) => onContentChange(event.target.value)}
          onKeyDown={handleKeyDown}
        />
        <div className="chat-composer__toolbar">
          <button className="composer-button" type="button" aria-label="Attach context">
            <AttachIcon />
          </button>
          <label className="sr-only" htmlFor="companion-model">
            Response model
          </label>
          <select
            className="chat-composer__model"
            id="companion-model"
            value={modelPreferenceValue(modelPreference)}
            disabled={isLoading || isSending}
            onChange={(event) =>
              onModelPreferenceChange(modelPreferenceFromValue(event.target.value))
            }
          >
            <option value="inherit">Default · {inheritedModelLabel}</option>
            <option value="test">Test stream</option>
            {configuredModels.map((model) => (
              <option key={model.id} value={`configured:${model.id}`}>
                {model.displayName}
              </option>
            ))}
          </select>
          <button
            className="composer-send"
            type="submit"
            aria-label="Send message"
            disabled={isLoading || isSending || content.trim().length === 0}
          >
            <SendIcon />
          </button>
        </div>
      </form>
      <p className="composer-note" id="composer-note">
        {error ?? notice ?? "Your conversations stay in your private workspace."}
      </p>
    </main>
  );
}
