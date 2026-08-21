import { useState, type FormEvent, type KeyboardEvent } from "react";

import type { ChatMessage } from "../features/chat/types";
import type { ConfiguredModel } from "../features/models/configuredModels/types";
import { EmptyState } from "./EmptyState";

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
  messages: ChatMessage[];
  isLoading: boolean;
  isSending: boolean;
  error: string | null;
  configuredModels: ConfiguredModel[];
  selectedModelId: string | null;
  onModelSelect: (modelId: string | null) => void;
  onSend: (content: string) => Promise<void>;
}

export function ChatSurface({
  messages,
  isLoading,
  isSending,
  error,
  configuredModels,
  selectedModelId,
  onModelSelect,
  onSend,
}: ChatSurfaceProps) {
  const [content, setContent] = useState("");
  const hasMessages = messages.length > 0;

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const message = content.trim();
    if (!message || isSending) return;
    setContent("");
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
        <section className="chat-thread" aria-label="Conversation messages" aria-live="polite">
          {messages.map((message) => (
            <article
              className={`chat-message chat-message--${message.role}`}
              key={message.id}
            >
              <p>{message.content || (message.status === "streaming" ? "Thinking…" : "")}</p>
              {message.errorMessage ? (
                <span className="chat-message__error">{message.errorMessage}</span>
              ) : null}
            </article>
          ))}
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
          onChange={(event) => setContent(event.target.value)}
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
            value={selectedModelId ?? ""}
            disabled={isLoading || isSending}
            onChange={(event) => onModelSelect(event.target.value || null)}
          >
            <option value="">Test stream</option>
            {configuredModels.map((model) => (
              <option key={model.id} value={model.id}>
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
        {error ?? "Your conversations stay in your private workspace."}
      </p>
    </main>
  );
}
