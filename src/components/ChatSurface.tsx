import {
  Fragment,
  memo,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  type ClipboardEvent,
  type FormEvent,
  type KeyboardEvent,
  type RefObject,
} from "react";

import { onConversationScrollToEnd } from "../features/chat/chatScrollEvents";
import {
  isAttachableImage,
  readClipboardImages,
} from "../features/chat/imageAttachments";
import type {
  ChatMessage,
  PendingAttachment,
  ToolCallChipItem,
} from "../features/chat/types";
import { CompanionSelect } from "../features/companions/CompanionSelect";
import { companionLabel, type Companion } from "../features/companions/types";
import type { MemoryRecallChipData } from "../features/memory";
import { EmptyState } from "./EmptyState";
import { MarkdownRenderer } from "./MarkdownRenderer";
import {
  CallTranscriptError,
  CallTranscriptItem,
  useConversationCalls,
  type CallThread,
  type StreamingCallMessage,
} from "../features/calls";
import { MemoryRecallChip } from "./MemoryRecallChip";
import { ToolCallChip } from "./ToolCallChip";
import { EmojiPicker } from "./EmojiPicker/EmojiPicker";
import { ReasoningDisclosure } from "./ReasoningDisclosure";

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
  /** Provider-supplied reasoning per assistant message — live-session only. */
  reasoningByMessageId: Record<string, string>;
  content: string;
  /** Composer images awaiting send. */
  pendingAttachments: PendingAttachment[];
  /** The roster the composer picks from — who you are talking to. */
  companions: Companion[];
  companionId: string | null;
  onContentChange: (content: string) => void;
  onCompanionChange: (companionId: string) => void;
  onSend: (content: string) => Promise<void>;
  onAttachFiles: (files: File[]) => void;
  onRemoveAttachment: (attachmentId: string) => void;
}

interface CallPlacements {
  beforeFirstMessage: CallThread[];
  afterMessageId: Map<string, CallThread[]>;
}

/** Keep chat sequence authoritative and place each call after the latest
 * message that already existed when the call opened. The call's createdAt is
 * immutable, so later call turns update the same slot instead of moving it. */
function placeCalls(messages: ChatMessage[], threads: CallThread[]): CallPlacements {
  const beforeFirstMessage: CallThread[] = [];
  const afterMessageId = new Map<string, CallThread[]>();
  const oldestFirst = [...threads].sort(
    (left, right) =>
      left.call.createdAt - right.call.createdAt || left.call.id.localeCompare(right.call.id),
  );

  for (const thread of oldestFirst) {
    let anchorId: string | null = null;
    for (const message of messages) {
      if (message.createdAt <= thread.call.createdAt) anchorId = message.id;
    }

    if (!anchorId) {
      beforeFirstMessage.push(thread);
      continue;
    }
    const anchored = afterMessageId.get(anchorId) ?? [];
    anchored.push(thread);
    afterMessageId.set(anchorId, anchored);
  }

  return { beforeFirstMessage, afterMessageId };
}

/** How far from the true bottom still counts as "at the bottom". Wide enough
 * to absorb fractional-pixel layout drift, narrow enough that a deliberate
 * scroll-away is respected. */
const PINNED_TO_END_SLACK_PX = 48;

interface ChatThreadProps {
  threadRef: RefObject<HTMLElement | null>;
  /** True while the reader belongs at the bottom — armed by every explicit
   * jump-to-end, released and re-armed by their own scrolling. */
  pinnedToEndRef: RefObject<boolean>;
  messages: ChatMessage[];
  recallByMessageId: Record<string, MemoryRecallChipData>;
  toolCallsByMessageId: Record<string, ToolCallChipItem[]>;
  reasoningByMessageId: Record<string, string>;
  callThreads: CallThread[];
  streamingCallMessages: StreamingCallMessage[];
  callsError: string | null;
  companions: Companion[];
}

/** The transcript, behind memo. Typing routes every keystroke through the
 * store draft and back down through ChatSurface, so without this wall the
 * whole conversation — every markdown row, every base64 image src — was
 * rebuilt per character, and the composer got slower the longer the chat
 * grew. Every prop here is reference-stable while the user types: message
 * state comes straight from the store, call state from useState inside
 * useConversationCalls. The wall holds only as long as that stays true. */
const ChatThread = memo(function ChatThread({
  threadRef,
  pinnedToEndRef,
  messages,
  recallByMessageId,
  toolCallsByMessageId,
  reasoningByMessageId,
  callThreads,
  streamingCallMessages,
  callsError,
  companions,
}: ChatThreadProps) {
  const callPlacements = useMemo(
    () => placeCalls(messages, callThreads),
    [messages, callThreads],
  );
  const callAgentNames = useMemo(
    () => new Map(companions.map((companion) => [companion.id, companionLabel(companion)])),
    [companions],
  );

  // The pin stays honest through the scroll events themselves: every scroll —
  // the reader's wheel and our own snaps alike — re-measures whether the view
  // sits at the bottom. Scrolling up releases it; coming back re-arms it.
  useEffect(() => {
    const thread = threadRef.current;
    if (!thread) return;
    const measure = () => {
      pinnedToEndRef.current =
        thread.scrollHeight - thread.scrollTop - thread.clientHeight <=
        PINNED_TO_END_SLACK_PX;
    };
    thread.addEventListener("scroll", measure, { passive: true });
    return () => thread.removeEventListener("scroll", measure);
  }, [threadRef, pinnedToEndRef]);

  // No dependency array on purpose: this component sits behind memo, so a
  // commit here IS the transcript changing — a thinking row, streamed tokens,
  // a call turn. While the reader is pinned, land on the new bottom before
  // paint. Instant, never smooth: a per-token smooth scroll cancels its own
  // last animation and spends the whole stream easing in from zero velocity —
  // the crawl that kept the viewport parked up top while the companion thought.
  useLayoutEffect(() => {
    const thread = threadRef.current;
    if (thread && pinnedToEndRef.current) thread.scrollTop = thread.scrollHeight;
  });

  return (
    <section
      className="chat-thread"
      ref={threadRef}
      aria-label="Conversation messages"
      aria-live="polite"
    >
      {callPlacements.beforeFirstMessage.map((thread) => (
        <CallTranscriptItem
          key={thread.call.id}
          thread={thread}
          agentNames={callAgentNames}
          streamingMessages={streamingCallMessages.filter(
            (message) => message.callId === thread.call.id,
          )}
        />
      ))}
      {messages.map((message) => {
        const recall = recallByMessageId[message.id];
        const toolCalls = toolCallsByMessageId[message.id];
        const reasoning = reasoningByMessageId[message.id] ?? "";
        const callsAfterMessage = callPlacements.afterMessageId.get(message.id) ?? [];
        const hasToolRow = Boolean(toolCalls && toolCalls.length > 0);
        const showReasoning =
          message.role === "assistant" &&
          (Boolean(reasoning) || (message.status === "streaming" && !message.content));
        // The first tool call opens its own row, like a message of its
        // own; later calls in the same turn join that row. The turn's
        // real reply only gets a row once it actually has something to
        // show — otherwise it's a redundant empty bubble under the chip.
        const showTextRow =
          !hasToolRow ||
          message.attachments.length > 0 ||
          Boolean(message.content) ||
          Boolean(message.errorMessage) ||
          Boolean(recall) ||
          showReasoning;
        return (
          <Fragment key={message.id}>
            {toolCalls && toolCalls.length > 0 ? (
              <article className="chat-message chat-message--assistant chat-message--tool-activity">
                <ToolCallChip calls={toolCalls} />
              </article>
            ) : null}
            {showTextRow ? (
              <article className={`chat-message chat-message--${message.role}`}>
                {message.attachments.length > 0 ? (
                  <div className="chat-message__images">
                    {message.attachments.map((attachment) => (
                      <img
                        key={attachment.id}
                        src={`data:${attachment.mediaType};base64,${attachment.data}`}
                        alt="Attached image"
                      />
                    ))}
                  </div>
                ) : null}
                {showReasoning ? (
                  <ReasoningDisclosure
                    reasoning={reasoning}
                    isStreaming={message.status === "streaming"}
                  />
                ) : null}
                {message.role === "assistant" && message.content ? (
                  <MarkdownRenderer content={message.content} />
                ) : message.content ? (
                  <p>{message.content}</p>
                ) : null}
                {message.errorMessage ? (
                  <span className="chat-message__error">{message.errorMessage}</span>
                ) : null}
                {recall ? <MemoryRecallChip data={recall} /> : null}
              </article>
            ) : null}
            {callsAfterMessage.map((thread) => (
              <CallTranscriptItem
                key={thread.call.id}
                thread={thread}
                agentNames={callAgentNames}
                streamingMessages={streamingCallMessages.filter(
                  (streaming) => streaming.callId === thread.call.id,
                )}
              />
            ))}
          </Fragment>
        );
      })}
      {callsError ? <CallTranscriptError error={callsError} /> : null}
    </section>
  );
});

export function ChatSurface({
  activeConversationId,
  messages,
  isLoading,
  isSending,
  error,
  notice,
  recallByMessageId,
  toolCallsByMessageId,
  reasoningByMessageId,
  content,
  pendingAttachments,
  companions,
  companionId,
  onContentChange,
  onCompanionChange,
  onSend,
  onAttachFiles,
  onRemoveAttachment,
}: ChatSurfaceProps) {
  const threadRef = useRef<HTMLElement>(null);
  const pinnedToEndRef = useRef(true);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const messageInputRef = useRef<HTMLTextAreaElement>(null);
  const positionedConversationIdRef = useRef<string | null>(null);
  const activeConversationIdRef = useRef(activeConversationId);
  activeConversationIdRef.current = activeConversationId;
  const hasMessages = messages.length > 0;
  const {
    threads: callThreads,
    streamingMessages: streamingCallMessages,
    isInitialLoading: areCallsInitiallyLoading,
    error: callsError,
  } = useConversationCalls(activeConversationId, isSending);
  // Opening a conversation is a snap, not a tour through its history. Wait
  // until BOTH independently loaded timelines (messages + calls) are in the
  // DOM, then land at the true bottom before paint. Images that finish decoding
  // a moment later get one corrective snap so they cannot shift it upward.
  useLayoutEffect(() => {
    if (!activeConversationId) {
      positionedConversationIdRef.current = null;
      return;
    }
    if (
      isLoading ||
      areCallsInitiallyLoading ||
      positionedConversationIdRef.current === activeConversationId
    ) {
      return;
    }

    const thread = threadRef.current;
    if (!thread) return;
    const conversationId = activeConversationId;
    const snapToEnd = () => {
      if (
        activeConversationIdRef.current === conversationId &&
        threadRef.current === thread
      ) {
        pinnedToEndRef.current = true;
        thread.scrollTop = thread.scrollHeight;
      }
    };

    positionedConversationIdRef.current = conversationId;
    snapToEnd();

    const pendingImages = Array.from(thread.querySelectorAll("img")).filter(
      (image) => !image.complete,
    );
    pendingImages.forEach((image) => image.addEventListener("load", snapToEnd));
    return () => {
      pendingImages.forEach((image) => image.removeEventListener("load", snapToEnd));
      // React's development StrictMode deliberately mounts effects twice.
      // Release the marker with the listeners so the second setup remains
      // complete rather than keeping the snap but losing image correction.
      if (positionedConversationIdRef.current === conversationId) {
        positionedConversationIdRef.current = null;
      }
    };
  }, [activeConversationId, areCallsInitiallyLoading, isLoading]);

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
        if (thread) {
          // An explicit "take me to the end" arms the pin, so the follow in
          // ChatThread keeps the view there as the turn grows. The jump is
          // instant — a smooth glide races the transcript growing underneath
          // it and lands short.
          pinnedToEndRef.current = true;
          thread.scrollTop = thread.scrollHeight;
        }
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
    if ((!message && pendingAttachments.length === 0) || isSending) return;
    await onSend(message);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      event.currentTarget.form?.requestSubmit();
    }
  };

  const handleEmojiSelect = (emoji: string) => {
    const input = messageInputRef.current;
    const selectionStart = input?.selectionStart ?? content.length;
    const selectionEnd = input?.selectionEnd ?? selectionStart;
    onContentChange(
      `${content.slice(0, selectionStart)}${emoji}${content.slice(selectionEnd)}`,
    );
    const caret = selectionStart + emoji.length;
    requestAnimationFrame(() => {
      input?.focus();
      input?.setSelectionRange(caret, caret);
    });
  };

  // A pasted screenshot is an attachment, not a wall of nothing. WebKitGTK
  // hands pasted images through `items` rather than `files` — and for a
  // copied screenshot it often hands an EMPTY DataTransfer, so when both
  // lists come up dry we go ask the async clipboard API directly.
  const handlePaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const fromItems = Array.from(event.clipboardData.items)
      .filter((item) => item.kind === "file")
      .map((item) => item.getAsFile())
      .filter((file): file is File => file !== null);
    const images = [...event.clipboardData.files, ...fromItems]
      .filter(isAttachableImage)
      // The same screenshot can appear in both lists — keep each image once.
      .filter(
        (file, index, all) =>
          all.findIndex(
            (other) =>
              other.name === file.name &&
              other.size === file.size &&
              other.type === file.type,
          ) === index,
      );
    if (images.length > 0) {
      event.preventDefault();
      onAttachFiles(images);
      return;
    }
    // No preventDefault here: if the clipboard holds text, the normal paste
    // proceeds; if it holds only an image, this fallback catches it.
    void readClipboardImages().then((files) => {
      if (files.length > 0) onAttachFiles(files);
    });
  };

  return (
    <main className={`chat-surface ${hasMessages ? "has-messages" : ""}`} id="chat">
      {hasMessages ? (
        <ChatThread
          threadRef={threadRef}
          pinnedToEndRef={pinnedToEndRef}
          messages={messages}
          recallByMessageId={recallByMessageId}
          toolCallsByMessageId={toolCallsByMessageId}
          reasoningByMessageId={reasoningByMessageId}
          callThreads={callThreads}
          streamingCallMessages={streamingCallMessages}
          callsError={callsError}
          companions={companions}
        />
      ) : (
        <EmptyState />
      )}

      <form className="chat-composer" onSubmit={handleSubmit}>
        <label className="sr-only" htmlFor="companion-message">
          Message Companion
        </label>
        {pendingAttachments.length > 0 ? (
          <div className="chat-composer__attachments">
            {pendingAttachments.map((attachment) => (
              <span className="composer-attachment" key={attachment.id}>
                <img
                  src={`data:${attachment.mediaType};base64,${attachment.data}`}
                  alt="Image ready to send"
                />
                <button
                  type="button"
                  aria-label="Remove image"
                  onClick={() => onRemoveAttachment(attachment.id)}
                >
                  ×
                </button>
              </span>
            ))}
          </div>
        ) : null}
        <textarea
          ref={messageInputRef}
          id="companion-message"
          name="message"
          rows={1}
          placeholder="Message Companion…"
          aria-describedby="composer-note"
          value={content}
          disabled={isLoading}
          onChange={(event) => onContentChange(event.target.value)}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
        />
        <div className="chat-composer__toolbar">
          <input
            ref={fileInputRef}
            className="sr-only"
            type="file"
            accept="image/png,image/jpeg,image/webp,image/gif"
            multiple
            tabIndex={-1}
            aria-hidden="true"
            onChange={(event) => {
              const files = Array.from(event.target.files ?? []);
              event.target.value = "";
              if (files.length > 0) onAttachFiles(files);
            }}
          />
          <button
            className="composer-button"
            type="button"
            aria-label="Attach images"
            disabled={isLoading || isSending}
            onClick={() => fileInputRef.current?.click()}
          >
            <AttachIcon />
          </button>
          <EmojiPicker
            triggerClassName="composer-button"
            disabled={isLoading || isSending}
            returnFocus={false}
            onSelect={handleEmojiSelect}
          />
          <label className="sr-only" htmlFor="companion-picker">
            Companion
          </label>
          <CompanionSelect
            className="chat-composer__model"
            id="companion-picker"
            companions={companions}
            value={companionId}
            disabled={isLoading || isSending}
            onChange={onCompanionChange}
          />
          <button
            className="composer-send"
            type="submit"
            aria-label="Send message"
            disabled={
              isLoading ||
              isSending ||
              (content.trim().length === 0 && pendingAttachments.length === 0)
            }
          >
            <SendIcon />
          </button>
        </div>
      </form>
      <p
        className={`composer-note${error ? " composer-note--error" : ""}`}
        id="composer-note"
        role={error ? "alert" : undefined}
      >
        {error ?? notice ?? "Your conversations stay in your private workspace."}
      </p>
    </main>
  );
}
