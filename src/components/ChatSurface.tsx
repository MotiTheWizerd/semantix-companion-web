import {
  Fragment,
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
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
import { CompanionMark } from "./CompanionMark";
import { EmptyState } from "./EmptyState";
import { MarkdownRenderer } from "./MarkdownRenderer";
import {
  CallTranscriptError,
  CallTranscriptItem,
  useConversationCalls,
  type CallSegment,
  type CallThread,
  type RavenCallMessage,
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

/** The stop square — the same orb as send, carrying a stop while the
 * companion works. */
function StopIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <rect x="5.75" y="5.75" width="8.5" height="8.5" rx="2" />
    </svg>
  );
}

interface ChatSurfaceProps {
  activeConversationId: string | null;
  messages: ChatMessage[];
  isLoading: boolean;
  isSending: boolean;
  /** A memory tool is running — the presence line says "remembering". */
  isRemembering: boolean;
  error: string | null;
  notice: string | null;
  /** 🧠 chip data per sent user message — live-session only. */
  recallByMessageId: Record<string, MemoryRecallChipData>;
  /** 📖 tool chips per assistant message — live-session only. */
  toolCallsByMessageId: Record<string, ToolCallChipItem[]>;
  /** Provider-supplied reasoning per assistant message — live-session only. */
  reasoningByMessageId: Record<string, string>;
  /** The row whose thoughts are arriving now, if any — the only row whose
   * disclosure may pulse "Thinking…". */
  thinkingMessageId: string | null;
  content: string;
  /** Composer images awaiting send. */
  pendingAttachments: PendingAttachment[];
  /** The roster the composer picks from — who you are talking to. */
  companions: Companion[];
  companionId: string | null;
  onContentChange: (content: string) => void;
  onCompanionChange: (companionId: string) => void;
  onSend: (content: string) => Promise<void>;
  /** The stop square: end the turn in flight where it stands. */
  onStop: () => void;
  onAttachFiles: (files: File[]) => void;
  onRemoveAttachment: (attachmentId: string) => void;
}

interface CallPlacements {
  beforeFirstMessage: CallSegment[];
  afterMessageId: Map<string, CallSegment[]>;
}

/** Backstage rows — persisted for the MODEL, never shown to the person. A
 * `system` row is machinery talking to the companion (a wake notice, a call
 * record); a completed assistant row with nothing in it is a woken turn that
 * answered through the call instead of the thread. Both stay in the database
 * and ride every later request's history — but the call CARD is the whole
 * user-facing surface of a call, and showing the stage directions beside it
 * broke the illusion of a real call (Moti, s533: "it should feel like a real
 * call"). */
function isUserFacing(message: ChatMessage): boolean {
  if (message.role === "system") return false;
  if (
    message.role === "assistant" &&
    message.status === "completed" &&
    !message.content &&
    message.attachments.length === 0 &&
    !message.errorMessage
  ) {
    return false;
  }
  return true;
}

/** The latest message that already existed at `at` — null before the first. */
function anchorAt(messages: ChatMessage[], at: number): string | null {
  let anchorId: string | null = null;
  for (const message of messages) {
    if (message.createdAt <= at) anchorId = message.id;
  }
  return anchorId;
}

/** Keep chat sequence authoritative and place each piece of a call after the
 * latest message that already existed when that piece happened.
 *
 * ⚑ A CALL IS PLACED TURN BY TURN, NOT AS ONE BLOCK. The opening sits where
 * the call was placed; each later turn sits where it arrived. Anchoring the
 * whole call at its opening pulled a reply that came seconds later ABOVE
 * text the companion had written in between (seen live, s571: "Done! I've
 * opened a call" pushed under Rook's answer to it). Turns that share an
 * anchor form one segment; a call whose turns straddle the companion's next
 * words shows as an opening and one or more continuations, each in its
 * place. Timestamps are immutable, so a refetch never moves a piece. */
function placeCalls(messages: ChatMessage[], threads: CallThread[]): CallPlacements {
  const beforeFirstMessage: CallSegment[] = [];
  const afterMessageId = new Map<string, CallSegment[]>();
  const oldestFirst = [...threads].sort(
    (left, right) =>
      left.call.createdAt - right.call.createdAt || left.call.id.localeCompare(right.call.id),
  );
  const place = (anchorId: string | null, segment: CallSegment) => {
    if (!anchorId) {
      beforeFirstMessage.push(segment);
      return;
    }
    const anchored = afterMessageId.get(anchorId) ?? [];
    anchored.push(segment);
    afterMessageId.set(anchorId, anchored);
  };

  for (const thread of oldestFirst) {
    const turns = [...thread.messages].sort(
      (left, right) => left.createdAt - right.createdAt || left.id.localeCompare(right.id),
    );
    // Cut at every change of anchor. A turn never lands before the opening:
    // it was said after the call was placed, so its anchor is at or past it.
    const cuts: { anchorId: string | null; messages: RavenCallMessage[] }[] = [
      { anchorId: anchorAt(messages, thread.call.createdAt), messages: [] },
    ];
    for (const turn of turns) {
      const anchorId = anchorAt(messages, turn.createdAt);
      const current = cuts[cuts.length - 1];
      if (anchorId === current.anchorId) current.messages.push(turn);
      else cuts.push({ anchorId, messages: [turn] });
    }
    cuts.forEach((cut, index) =>
      place(cut.anchorId, {
        thread,
        messages: cut.messages,
        isOpening: index === 0,
        isLatest: index === cuts.length - 1,
      }),
    );
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
  thinkingMessageId: string | null;
  callThreads: CallThread[];
  streamingCallMessages: StreamingCallMessage[];
  /** callId → agentId while a woken reply is in flight — changes only on wake
   * edges, never per keystroke, so it is safe inside the memo wall. */
  replyingByCallId: ReadonlyMap<string, string>;
  callsError: string | null;
  companions: Companion[];
  /** A turn is in flight — the presence line sits at the thread's end. */
  isSending: boolean;
  isRemembering: boolean;
  /** Who is working, for the presence line's sentence. */
  companionName: string;
  /** Who the thread answers to NOW. A row frozen to a different companion is
   *  bylined, so a picker switch mid-thread reads as a change of speaker
   *  rather than silently re-attributing what was already said (s564). */
  companionId: string | null;
}

/** How long after the last streamed token the companion still counts as
 * writing. Past it the pause reads as thought — which is what it is. */
const WRITING_GRACE_MS = 1500;

type PresenceVerb = "thinking" | "remembering" | "working" | "writing";

interface PresenceLineProps {
  name: string;
  /** The companion's picture, when it has one — the orb wears the same face
   *  as the tab and the picker, or its lettermark. */
  avatarUrl?: string | null;
  isRemembering: boolean;
  messages: ChatMessage[];
  toolCallsByMessageId: Record<string, ToolCallChipItem[]>;
}

/** What kind of work the companion is doing right now, from what the store
 * can see. Remembering outranks everything (memory tools never light a chip,
 * so this verb is the only place a person sees the companion remember); a
 * running tool is working; fresh text is writing; the rest is thinking. */
function presenceVerb({
  isRemembering,
  messages,
  toolCallsByMessageId,
  now,
}: PresenceLineProps & { now: number }): PresenceVerb {
  if (isRemembering) return "remembering";
  const working = Object.values(toolCallsByMessageId).some((calls) =>
    calls.some((call) => call.status === "running"),
  );
  if (working) return "working";
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role !== "assistant" || message.status !== "streaming") continue;
    // updatedAt is stamped by the store on every delta, so this is "how long
    // since the last token" — not the row's age.
    if (message.content.length > 0 && now - message.updatedAt < WRITING_GRACE_MS) {
      return "writing";
    }
    break;
  }
  return "thinking";
}

/** The companion, visibly at work: one line at the thread's end for the
 * whole of a turn, saying what kind of work. It is the answer to "is it
 * still going?" — while this is here, it is; the moment the turn lands, it
 * is gone. The clock ticks only while the line is mounted. */
function PresenceLine({
  name,
  avatarUrl,
  isRemembering,
  messages,
  toolCallsByMessageId,
}: PresenceLineProps) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(timer);
  }, []);
  const verb = presenceVerb({ name, isRemembering, messages, toolCallsByMessageId, now });
  return (
    <article className={`chat-message chat-message--assistant chat-presence chat-presence--${verb}`}>
      <span className="chat-presence__orb">
        <CompanionMark src={avatarUrl} name={name} />
      </span>
      <span className="chat-presence__text">
        {name} is {verb}…
      </span>
    </article>
  );
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
  thinkingMessageId,
  callThreads,
  streamingCallMessages,
  replyingByCallId,
  callsError,
  companions,
  isSending,
  isRemembering,
  companionName,
  companionId,
}: ChatThreadProps) {
  const contentRef = useRef<HTMLDivElement>(null);
  // The thread's current companion, resolved the way the backend resolves an
  // unpicked thread: to the built-in one. Rows stamped with anyone else were
  // said by a different speaker and are bylined as such.
  const currentCompanionId = useMemo(
    () =>
      companionId ??
      companions.find((candidate) => candidate.isBuiltIn)?.id ??
      null,
    [companions, companionId],
  );
  // Calls anchor against the same filtered list the reader sees, so a card
  // whose nearest real anchor is a hidden row lands after the latest VISIBLE
  // message instead of vanishing with its anchor.
  const visibleMessages = useMemo(() => messages.filter(isUserFacing), [messages]);
  const callPlacements = useMemo(
    () => placeCalls(visibleMessages, callThreads),
    [visibleMessages, callThreads],
  );
  const callAgentNames = useMemo(
    () => new Map(companions.map((companion) => [companion.id, companionLabel(companion)])),
    [companions],
  );

  // Faces for the call card, kept separate from the names rather than folded
  // into one map: only some companions have a picture, and every other reader
  // of `callAgentNames` (the tool chips) wants the name and nothing else.
  const callAgentAvatars = useMemo(
    () =>
      new Map(
        companions
          .filter((companion): companion is typeof companion & { avatarUrl: string } =>
            Boolean(companion.avatarUrl),
          )
          .map((companion) => [companion.id, companion.avatarUrl]),
      ),
    [companions],
  );

  // Open/closed per call, held here because a call can be several rows:
  // its opening and its continuations share one answer, so a press on any
  // of them opens or closes the whole call. Closed by default; a segment
  // with something live asks to open.
  const [expandedCallIds, setExpandedCallIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const setCallExpanded = useCallback((callId: string, expanded: boolean) => {
    setExpandedCallIds((current) => {
      if (current.has(callId) === expanded) return current;
      const next = new Set(current);
      if (expanded) next.add(callId);
      else next.delete(callId);
      return next;
    });
  }, []);
  const renderCallSegment = (segment: CallSegment) => (
    <CallTranscriptItem
      key={segment.thread.call.id}
      segment={segment}
      expanded={expandedCallIds.has(segment.thread.call.id)}
      onExpandedChange={setCallExpanded}
      agentNames={callAgentNames}
      agentAvatars={callAgentAvatars}
      streamingMessages={streamingCallMessages.filter(
        (message) => message.callId === segment.thread.call.id,
      )}
      replyingAgentId={replyingByCallId.get(segment.thread.call.id) ?? null}
    />
  );

  // ⚑ NOT EVERY GROWTH IS A COMMIT OF THIS COMPONENT. The follow below rides
  // ChatThread's own renders, which covers new messages and streamed tokens —
  // but a call card owns its `expanded` state privately and opens ITSELF the
  // moment speech starts, so the whole turns block appears, and grows through
  // the back-and-forth, without ChatThread ever re-rendering. The view is left
  // behind with nothing to tell it.
  // Enumerating growth paths is the trap that cost us the thinking panel, so
  // this watches the RESULT instead: any change in content height re-lands the
  // pin while it is armed, whoever caused it — a call expanding, an image
  // decoding, a card we have not written yet. Reader scrolled away ⇒ pin
  // released ⇒ this does nothing, which is the whole contract.
  useEffect(() => {
    const thread = threadRef.current;
    const content = contentRef.current;
    if (!thread || !content) return;
    const observer = new ResizeObserver(() => {
      if (pinnedToEndRef.current) thread.scrollTop = thread.scrollHeight;
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [threadRef, pinnedToEndRef]);

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
      <div className="chat-thread__content" ref={contentRef}>
        {callPlacements.beforeFirstMessage.map(renderCallSegment)}
        {visibleMessages.map((message) => {
          const recall = recallByMessageId[message.id];
          const toolCalls = toolCallsByMessageId[message.id] ?? [];
          // A row's tool calls sit where they happened: the ones made before
          // the companion said anything go above its text, the ones made
          // after it spoke go below — and what it says next is the next row.
          // (Memory tools never arrive here: remembering is not tool use.)
          const toolsBefore = toolCalls.filter((call) => !call.afterText);
          const toolsAfter = toolCalls.filter((call) => call.afterText);
          const reasoning = reasoningByMessageId[message.id] ?? "";
          const callsAfterMessage = callPlacements.afterMessageId.get(message.id) ?? [];
          const hasToolRow = toolCalls.length > 0;
          // The disclosure exists only once the provider has actually sent a
          // thought. Waiting for the first token is not thinking, and the
          // presence line already says the companion is working.
          const showReasoning = message.role === "assistant" && Boolean(reasoning);
          // A tool call opens its own row, like a message of its own; later
          // calls on the same side of the text join that row. The text row
          // only appears once it actually has something to show — otherwise
          // it's a redundant empty bubble beside the chip.
          const showTextRow =
            !hasToolRow ||
            message.attachments.length > 0 ||
            Boolean(message.content) ||
            Boolean(message.errorMessage) ||
            Boolean(recall) ||
            showReasoning;
          // Spoken by someone other than who the thread answers to now — the
          // row keeps its speaker, and says so.
          const bylineCompanionId =
            message.role === "assistant" &&
            message.companionId &&
            message.companionId !== currentCompanionId
              ? message.companionId
              : null;
          return (
            <Fragment key={message.id}>
              {toolsBefore.length > 0 ? (
                <article className="chat-message chat-message--assistant chat-message--tool-activity">
                  <ToolCallChip calls={toolsBefore} agentNames={callAgentNames} />
                </article>
              ) : null}
              {showTextRow ? (
                <article className={`chat-message chat-message--${message.role}`}>
                  {bylineCompanionId ? (
                    <span className="chat-message__byline">
                      <CompanionMark
                        src={callAgentAvatars.get(bylineCompanionId)}
                        name={callAgentNames.get(bylineCompanionId)}
                      />
                      {callAgentNames.get(bylineCompanionId) ?? "Another companion"}
                    </span>
                  ) : null}
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
                      isThinking={thinkingMessageId === message.id}
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
              {toolsAfter.length > 0 ? (
                <article className="chat-message chat-message--assistant chat-message--tool-activity">
                  <ToolCallChip calls={toolsAfter} agentNames={callAgentNames} />
                </article>
              ) : null}
              {callsAfterMessage.map(renderCallSegment)}
            </Fragment>
          );
        })}
        {isSending ? (
          <PresenceLine
            name={companionName}
            avatarUrl={currentCompanionId ? callAgentAvatars.get(currentCompanionId) : null}
            isRemembering={isRemembering}
            messages={messages}
            toolCallsByMessageId={toolCallsByMessageId}
          />
        ) : null}
        {callsError ? <CallTranscriptError error={callsError} /> : null}
      </div>
    </section>
  );
});

export function ChatSurface({
  activeConversationId,
  messages,
  isLoading,
  isSending,
  isRemembering,
  error,
  notice,
  recallByMessageId,
  toolCallsByMessageId,
  reasoningByMessageId,
  thinkingMessageId,
  content,
  pendingAttachments,
  companions,
  companionId,
  onContentChange,
  onCompanionChange,
  onSend,
  onStop,
  onAttachFiles,
  onRemoveAttachment,
}: ChatSurfaceProps) {
  const threadRef = useRef<HTMLElement>(null);
  const surfaceRef = useRef<HTMLElement>(null);
  const dockRef = useRef<HTMLDivElement>(null);
  const pinnedToEndRef = useRef(true);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const messageInputRef = useRef<HTMLTextAreaElement>(null);
  const positionedConversationIdRef = useRef<string | null>(null);
  const activeConversationIdRef = useRef(activeConversationId);
  activeConversationIdRef.current = activeConversationId;
  const {
    threads: callThreads,
    streamingMessages: streamingCallMessages,
    replyingByCallId,
    isInitialLoading: areCallsInitiallyLoading,
    error: callsError,
  } = useConversationCalls(activeConversationId, isSending);
  // The presence line names who is working: the tab's companion, else the
  // built-in one (an unpicked thread already answers to it).
  const companionName = useMemo(() => {
    const companion =
      companions.find((candidate) => candidate.id === companionId) ??
      companions.find((candidate) => candidate.isBuiltIn);
    return companion ? companionLabel(companion) : "Companion";
  }, [companions, companionId]);
  // No companion change mid-conversation (s569): a conversation exists only
  // once its first message has landed, and from then on it is with whoever
  // answered it. The picker is free on an empty tab and closed on a thread —
  // Rust refuses the switch too; this just keeps the control honest.
  const isCompanionSettled = activeConversationId !== null;
  // A thread whose every row is backstage still shows its call cards — the
  // cards are the one surface a call is allowed to have.
  const hasMessages = messages.some(isUserFacing) || callThreads.length > 0;

  // The dock floats OVER the thread so messages frost under the composer glass.
  // That costs the thread its floor, so the thread rents it back as bottom
  // padding — and the dock's height is not a constant: the textarea grows to
  // 160px, attachment chips appear, the note wraps on an error. Measure it and
  // publish it as --composer-dock-h. A dock that grew while the reader was
  // pinned would push the newest line up behind the glass, so the same callback
  // re-lands the pin.
  useLayoutEffect(() => {
    const dock = dockRef.current;
    const surface = surfaceRef.current;
    if (!dock || !surface) return;
    const observer = new ResizeObserver(() => {
      surface.style.setProperty("--composer-dock-h", `${dock.offsetHeight}px`);
      const thread = threadRef.current;
      if (thread && pinnedToEndRef.current) thread.scrollTop = thread.scrollHeight;
    });
    observer.observe(dock);
    return () => observer.disconnect();
  }, []);

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
    <main
      className={`chat-surface ${hasMessages ? "has-messages" : ""}`}
      id="chat"
      ref={surfaceRef}
    >
      {hasMessages ? (
        <ChatThread
          threadRef={threadRef}
          pinnedToEndRef={pinnedToEndRef}
          messages={messages}
          recallByMessageId={recallByMessageId}
          toolCallsByMessageId={toolCallsByMessageId}
          reasoningByMessageId={reasoningByMessageId}
          thinkingMessageId={thinkingMessageId}
          callThreads={callThreads}
          streamingCallMessages={streamingCallMessages}
          replyingByCallId={replyingByCallId}
          callsError={callsError}
          companions={companions}
          isSending={isSending}
          isRemembering={isRemembering}
          companionName={companionName}
          companionId={companionId}
        />
      ) : (
        <EmptyState />
      )}

      <div className="chat-composer-dock" ref={dockRef}>
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
              disabled={isLoading || isSending || isCompanionSettled}
              title={
                isCompanionSettled
                  ? `This conversation is with ${companionName}. Start a new conversation to talk with someone else.`
                  : undefined
              }
              onChange={onCompanionChange}
            />
            {isSending ? (
              <button
                className="composer-send composer-send--stop"
                type="button"
                aria-label="Stop the companion"
                title="Stop"
                onClick={onStop}
              >
                <StopIcon />
              </button>
            ) : (
              <button
                className="composer-send"
                type="submit"
                aria-label="Send message"
                disabled={
                  isLoading ||
                  (content.trim().length === 0 && pendingAttachments.length === 0)
                }
              >
                <SendIcon />
              </button>
            )}
          </div>
        </form>
        <p
          className={`composer-note${error ? " composer-note--error" : ""}`}
          id="composer-note"
          role={error ? "alert" : undefined}
        >
          {error ?? notice ?? "Your conversations stay in your private workspace."}
        </p>
      </div>
    </main>
  );
}
