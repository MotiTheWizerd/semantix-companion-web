import type { UnlistenFn } from "@tauri-apps/api/event";
import { create } from "zustand";

import {
  getConversationThread,
  listConversations,
  onConversationTitled,
  onWokenChatEvent,
  stopTurn as requestStopTurn,
  submitMessage,
  updateConversationCompanion,
} from "../chat/chatService";
import { requestConversationScrollToEnd } from "../chat/chatScrollEvents";
import type {
  AcceptedMessage,
  ChatEvent,
  ChatMessage,
  Conversation,
  PendingAttachment,
  ToolCallChipItem,
} from "../chat/types";
import {
  MAX_ATTACHMENTS_PER_MESSAGE,
  prepareImageAttachment,
} from "../chat/imageAttachments";
import {
  listCompanions,
  onCompanionsChanged,
  reconcileCompanionEvent,
} from "../companions/companionService";
import type { Companion } from "../companions/types";
import {
  listConfiguredModels,
  onModelsChanged,
} from "../models/configuredModels/modelService";
import type { ConfiguredModel } from "../models/configuredModels/types";
import { runMemoryPreSend, type MemoryRecallChipData } from "../memory/preSend";
import { autoSleepAgent, onMemorySlept, sleepConversation } from "../memory/sleepService";
import { useNotificationsStore } from "../notifications/notificationsStore";
import {
  getUserPreferences,
  onUserPreferencesChanged,
  updateUserPreferences,
} from "../preferences/preferenceService";
import type { ModelPreference, UserPreferences } from "../preferences/types";

/** The two whole-workspace views. Settings is NOT one of them any more
 *  (s573: "it's annoying to move between the settings and the tabs") — it is
 *  a tab in the strip, so a person can be halfway through a key and one click
 *  from the conversation, and back. */
export type WorkspaceView = "chat" | "memory";

/** The one settings tab's id — fixed, so opening Settings twice finds the
 *  same tab instead of growing a second. */
export const SETTINGS_TAB_ID = "settings";

export interface ConversationTab {
  id: string;
  /** What the tab shows. A "settings" tab carries the conversation fields
   *  empty and never a conversation — the shape stays one so every reader
   *  of the strip keeps working; only the face and the surface differ. */
  kind: "conversation" | "settings";
  conversationId: string | null;
  title: string;
  draft: string;
  /** Composer images awaiting send — cleared on send, restored on failure. */
  attachments: PendingAttachment[];
  /** Who this tab talks to. null = nothing picked yet, so the built-in
   *  companion answers — Rust resolves the same way. */
  companionId: string | null;
  unreadCount: number;
  error: string | null;
  /** Non-error status line (e.g. a /sleep outcome) for the composer note. */
  notice: string | null;
}

interface ConversationRuntime {
  messages: ChatMessage[];
  isLoading: boolean;
  isStreaming: boolean;
  /** The companion is remembering — a memory tool runs. Never a chip; the
   *  thread's presence line is where a person sees it. */
  isRemembering: boolean;
  error: string | null;
  /** 🧠 chip per sent user message — live-session instrument, not persisted,
   *  so it dies with the runtime entry (reload = no chips, by design). */
  recallByMessageId: Record<string, MemoryRecallChipData>;
  /** 📖 tool chips per assistant message — same live-session contract. */
  toolCallsByMessageId: Record<string, ToolCallChipItem[]>;
  /** Provider-supplied thoughts per assistant message. Runtime-only so they
   *  never become later conversation context. */
  reasoningByMessageId: Record<string, string>;
  /** The row whose thoughts are arriving RIGHT NOW — the only time the
   *  disclosure may say "Thinking…". Set by a reasoning delta, cleared by the
   *  first text after it, a tool event, or the turn's end. A wait for the
   *  first token is not thinking, and neither is a tool running. */
  thinkingMessageId: string | null;
}

interface CompanionStore {
  activeView: WorkspaceView;
  isInitialising: boolean;
  isInitialised: boolean;
  conversations: Conversation[];
  companions: Companion[];
  configuredModels: ConfiguredModel[];
  userPreferences: UserPreferences;
  preferenceError: string | null;
  tabOrder: string[];
  tabsById: Record<string, ConversationTab>;
  activeTabId: string | null;
  runtimeByConversationId: Record<string, ConversationRuntime>;
  submittingByTabId: Record<string, boolean>;
  /** conversationId → the title it just stopped wearing, for the length of
   *  the settle: the sidebar row and the tab swap the old name for the new
   *  one with a small motion and a single sweep of light, then this empties
   *  itself. A rename is news for a second, and then it is just the name. */
  settlingTitles: Record<string, string>;
  initialise: () => Promise<void>;
  dispose: () => void;
  setActiveView: (view: WorkspaceView) => void;
  openConversation: (conversationId: string) => Promise<void>;
  openNewConversation: () => void;
  /** Settings as a tab: opens it once, and after that just moves to it. */
  openSettings: () => void;
  setActiveTab: (tabId: string) => void;
  closeTab: (tabId: string) => void;
  setDraft: (tabId: string, draft: string) => void;
  /** Attach prepared images to a tab's composer (capped per message). */
  addAttachments: (tabId: string, attachments: PendingAttachment[]) => void;
  removeAttachment: (tabId: string, attachmentId: string) => void;
  /** Prepare raw files (downscale + encode) and attach them to the composer. */
  attachFiles: (tabId: string, files: (File | Blob)[]) => Promise<void>;
  setTabCompanion: (tabId: string, companionId: string) => Promise<void>;
  setUserDefaultModel: (
    preference: Exclude<ModelPreference, { mode: "inherit" }>,
  ) => Promise<void>;
  /** Rename the person using this install. An empty string clears the name.
   *  Rejects on a refused name so the form can keep the reader in the field. */
  setUserDisplayName: (name: string) => Promise<void>;
  sendMessage: (tabId: string, content: string) => Promise<void>;
  /** The composer's stop square: end the tab's turn where it stands. */
  stopTurn: (tabId: string) => Promise<void>;
  /** The /sleep pass: distill the tab's conversation into long-term memory. */
  sleepActiveConversation: (tabId: string) => Promise<void>;
}

const EMPTY_USER_PREFERENCES: UserPreferences = {
  defaultModel: { mode: "test" },
  displayName: null,
  defaultCompanionId: null,
  updatedAt: 0,
};

/** How long a renamed row keeps its old name in hand — long enough for the
 *  swap and the sweep of light to play out (see shell/settle.css), no longer. */
const TITLE_SETTLE_MS = 1400;

let unlisteners: UnlistenFn[] = [];

/** Tabs whose person pressed stop while their send was still on its way to
 *  Rust (the memory pass runs first). sendMessage honours the wish before it
 *  submits; the turn's end clears it. */
const stopRequests = new Set<string>();

function createTabId(prefix: "new" | "conversation", id?: string): string {
  return id ? `${prefix}:${id}` : `${prefix}:${crypto.randomUUID()}`;
}

/** A fresh tab, talking to `companionId` — the remembered pick — or, with
 *  null, to the built-in companion (Rust resolves an unpicked thread the
 *  same way). */
function newConversationTab(companionId: string | null = null): ConversationTab {
  return {
    id: createTabId("new"),
    kind: "conversation",
    conversationId: null,
    title: "New conversation",
    draft: "",
    attachments: [],
    companionId,
    unreadCount: 0,
    error: null,
    notice: null,
  };
}

function tabForConversation(conversation: Conversation): ConversationTab {
  return {
    id: createTabId("conversation", conversation.id),
    kind: "conversation",
    conversationId: conversation.id,
    title: conversation.title,
    draft: "",
    attachments: [],
    companionId: conversation.companionId,
    unreadCount: 0,
    error: null,
    notice: null,
  };
}

function settingsTab(): ConversationTab {
  return {
    id: SETTINGS_TAB_ID,
    kind: "settings",
    conversationId: null,
    title: "Settings",
    draft: "",
    attachments: [],
    companionId: null,
    unreadCount: 0,
    error: null,
    notice: null,
  };
}

function emptyRuntime(isLoading = false): ConversationRuntime {
  return {
    messages: [],
    isLoading,
    isStreaming: false,
    isRemembering: false,
    error: null,
    recallByMessageId: {},
    toolCallsByMessageId: {},
    reasoningByMessageId: {},
    thinkingMessageId: null,
  };
}

/** Upsert one tool-call lifecycle event into a message's chip list —
 *  "running" appends, "ok"/"error" replaces the running entry in place and
 *  closes its clock: the chip shows how long the call took, timed from the
 *  moment the store first saw it (the backend sends no timestamps). */
function reconcileToolCall(
  calls: ToolCallChipItem[],
  event: Omit<ToolCallChipItem, "startedAt" | "elapsedMs">,
  now: number,
): ToolCallChipItem[] {
  const index = calls.findIndex((call) => call.callId === event.callId);
  if (index < 0) return [...calls, { ...event, startedAt: now, elapsedMs: null }];
  return calls.map((call, at) => {
    if (at !== index) return call;
    const landed = event.status !== "running";
    return {
      ...event,
      startedAt: call.startedAt,
      elapsedMs: landed ? Math.max(0, now - call.startedAt) : null,
    };
  });
}

/** The turn is over, whatever ended it. A chip still "running" now never
 *  landed — a stop mid-tool drops the call with the turn — and closes as an
 *  error that says so; a remembering that never said it finished has
 *  finished (its "done" was dropped with the same turn). */
function settleRuntime(runtime: ConversationRuntime, now: number): ConversationRuntime {
  const toolCallsByMessageId = Object.fromEntries(
    Object.entries(runtime.toolCallsByMessageId).map(([messageId, calls]) => [
      messageId,
      calls.map((call) =>
        call.status === "running"
          ? {
              ...call,
              status: "error" as const,
              detail: "stopped",
              elapsedMs: Math.max(0, now - call.startedAt),
            }
          : call,
      ),
    ]),
  );
  return {
    ...runtime,
    isStreaming: false,
    isRemembering: false,
    thinkingMessageId: null,
    toolCallsByMessageId,
  };
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "Companion could not update this conversation.";
}

function reconcileConversation(
  conversations: Conversation[],
  conversation: Conversation,
): Conversation[] {
  return [conversation, ...conversations.filter((item) => item.id !== conversation.id)].sort(
    (left, right) => right.updatedAt - left.updatedAt,
  );
}

function reconcileMessage(messages: ChatMessage[], message: ChatMessage): ChatMessage[] {
  return [message, ...messages.filter((item) => item.id !== message.id)].sort(
    (left, right) => left.sequence - right.sequence,
  );
}

function acceptedEvent(accepted: AcceptedMessage): ChatEvent {
  return { kind: "accepted", ...accepted };
}

type StreamDelta = Extract<ChatEvent, { kind: "assistantDelta" | "assistantReasoningDelta" }>;

function isStreamDelta(event: ChatEvent): event is StreamDelta {
  return event.kind === "assistantDelta" || event.kind === "assistantReasoningDelta";
}

/** A frame's worth of deltas, neighbours of the same kind on the same row
 * joined into one. Order is kept exactly — a reasoning delta between two
 * text deltas stays between them — so folding changes how many times the
 * row is rewritten, never what it ends up saying. */
function foldStreamDeltas(deltas: StreamDelta[]): StreamDelta[] {
  const folded: StreamDelta[] = [];
  for (const delta of deltas) {
    const last = folded[folded.length - 1];
    if (last && last.kind === delta.kind && last.messageId === delta.messageId) {
      folded[folded.length - 1] = { ...last, delta: last.delta + delta.delta };
    } else {
      folded.push(delta);
    }
  }
  return folded;
}

/** Discrete moments only — send accepted, turn started, turn finished. The
 * continuous kinds (deltas, reasoning, tool chips) deliberately request
 * nothing: following a growing transcript belongs to ChatThread's bottom
 * pin. Requesting a scroll per delta made every animation cancel the last
 * one, so during a long think the viewport crawled instead of arriving —
 * and it also overrode a reader who had scrolled up on purpose. */
function requestScrollForChatEvent(event: ChatEvent): void {
  if (event.kind === "accepted") {
    requestConversationScrollToEnd(event.conversation.id);
  } else if (event.kind === "assistantStarted" || event.kind === "assistantCompleted") {
    requestConversationScrollToEnd(event.message.conversationId);
  }
}

/** Fold one woken-lane event into whatever runtime already holds its thread.
 *
 * The human lane replays these same shapes through its own channel with its
 * own optimistic bookkeeping; this fold is deliberately NARROWER — a woken
 * turn is BACKSTAGE (Moti, s533: "it should feel like a real call"), so only
 * its two durable moments land: `accepted` (the persisted row + the sidebar
 * reorder) and `assistantCompleted` (the finished reply). The live theatre —
 * deltas, reasoning, tool chips — is deliberately dropped: the call card owns
 * a call's liveness, and ChatSurface hides backstage rows anyway. The fold
 * never touches `isStreaming` (the composer belongs to the person), never
 * creates a runtime (an unopened thread loads complete on first open), and
 * ignores `failed` (a woken turn's error is the waker's log, not this tab's
 * banner). */
function foldWokenEvent(
  state: CompanionStore,
  event: ChatEvent,
): Partial<CompanionStore> {
  if (event.kind === "accepted") {
    const conversationId = event.conversation.id;
    const runtimeState = state.runtimeByConversationId[conversationId];
    return {
      conversations: reconcileConversation(state.conversations, event.conversation),
      ...(runtimeState
        ? {
            runtimeByConversationId: {
              ...state.runtimeByConversationId,
              [conversationId]: {
                ...runtimeState,
                messages: reconcileMessage(runtimeState.messages, event.message),
              },
            },
          }
        : {}),
    };
  }
  if (event.kind !== "assistantCompleted") return {};

  const conversationId = event.message.conversationId;
  const runtimeState = state.runtimeByConversationId[conversationId];
  if (!runtimeState) return {};
  // A silent woken reply (all of its words went through the call) never
  // renders, so it must not light an unread badge for a row nobody can see.
  const spoke = Boolean(event.message.content);
  const targetTab = Object.values(state.tabsById).find(
    (candidate) => candidate.conversationId === conversationId,
  );
  const isVisible = state.activeView === "chat" && targetTab?.id === state.activeTabId;
  return {
    ...(spoke && targetTab && !isVisible
      ? {
          tabsById: {
            ...state.tabsById,
            [targetTab.id]: { ...targetTab, unreadCount: targetTab.unreadCount + 1 },
          },
        }
      : {}),
    runtimeByConversationId: {
      ...state.runtimeByConversationId,
      [conversationId]: {
        ...runtimeState,
        messages: reconcileMessage(runtimeState.messages, event.message),
      },
    },
  };
}

export const useCompanionStore = create<CompanionStore>()((set, get) => ({
  activeView: "chat",
  isInitialising: false,
  isInitialised: false,
  conversations: [],
  companions: [],
  configuredModels: [],
  userPreferences: EMPTY_USER_PREFERENCES,
  preferenceError: null,
  tabOrder: [],
  tabsById: {},
  activeTabId: null,
  runtimeByConversationId: {},
  submittingByTabId: {},
  settlingTitles: {},

  initialise: async () => {
    if (get().isInitialising || get().isInitialised) return;
    set({ isInitialising: true });
    try {
      const [conversations, companions, configuredModels, userPreferences] =
        await Promise.all([
          listConversations(),
          listCompanions(),
          listConfiguredModels(),
          getUserPreferences(),
        ]);
      set({ conversations, companions, configuredModels, userPreferences });

      if (conversations[0]) {
        await get().openConversation(conversations[0].id);
      } else {
        get().openNewConversation();
      }

      const [stopModels, stopPreferences, stopCompanions, stopWokenEvents, stopSlept, stopTitled] =
        await Promise.all([
        onModelsChanged(() => {
          void Promise.all([
            listConfiguredModels(),
            listConversations(),
            getUserPreferences(),
          ]).then(([models, refreshedConversations, preferences]) => {
            const conversationsById = new Map(
              refreshedConversations.map((conversation) => [conversation.id, conversation]),
            );
            set((state) => ({
              configuredModels: models,
              conversations: refreshedConversations,
              userPreferences: preferences,
              tabsById: Object.fromEntries(
                Object.entries(state.tabsById).map(([tabId, tab]) => {
                  const conversation = tab.conversationId
                    ? conversationsById.get(tab.conversationId)
                    : undefined;
                  return [
                    tabId,
                    conversation
                      ? {
                          ...tab,
                          title: conversation.title,
                          companionId: conversation.companionId,
                        }
                      : tab,
                  ];
                }),
              ),
            }));
          });
        }),
        onUserPreferencesChanged((event) => {
          if (event.kind === "updated") set({ userPreferences: event.preferences });
        }),
        onCompanionsChanged((event) => {
          set((state) => {
            const companions = reconcileCompanionEvent(state.companions, event);
            if (event.kind !== "deleted") return { companions };
            // A deleted companion leaves its tabs pointing at nothing, which
            // is exactly what Rust reads as "the built-in one answers".
            return {
              companions,
              tabsById: Object.fromEntries(
                Object.entries(state.tabsById).map(([tabId, tab]) => [
                  tabId,
                  tab.companionId === event.companionId
                    ? { ...tab, companionId: null }
                    : tab,
                ]),
              ),
            };
          });
        }),
        onWokenChatEvent((event) => {
          set((state) => foldWokenEvent(state, event));
          // Scroll only for rows the reader can actually see: a hidden system
          // notice or a silent woken reply must never yank the viewport.
          if (event.kind === "accepted" && event.message.role !== "system") {
            requestConversationScrollToEnd(event.conversation.id);
          } else if (event.kind === "assistantCompleted" && event.message.content) {
            requestConversationScrollToEnd(event.message.conversationId);
          }
        }),
        // The sleeper's report: a quiet note when it kept something, silence
        // when the distiller read the turns and kept nothing — that is the
        // common case and not news. A failure is said once (the backend backs
        // off for ten minutes after one).
        onMemorySlept((event) => {
          const { notify } = useNotificationsStore.getState();
          if (event.kind === "failed") {
            notify({
              title: "💤 The sleeper",
              text: `Couldn't sleep on this conversation — ${event.message}`,
              status: "error",
            });
            return;
          }
          if (event.created + event.updated === 0) return;
          const names = event.memories.length ? ` — ${event.memories.join(", ")}` : "";
          const scribe = event.scribeNote ? ` · ${event.scribeNote}` : "";
          notify({
            title: "💤 Slept on its own",
            text:
              `${event.created} carved, ${event.updated} updated` +
              `${event.dropped ? `, ${event.dropped} dropped` : ""}${names}${scribe}`,
            status: "success",
          });
        }),
        // The titler's report: the thread's real name, once the background
        // model has read its opening. The sidebar entry and the tab rename
        // together; the row order does not move — a name is not activity.
        // The old name is kept for the length of the settle so the swap can
        // be seen happening, then forgotten.
        onConversationTitled((event) => {
          set((state) => {
            const previous =
              state.conversations.find((conversation) => conversation.id === event.conversationId)
                ?.title ?? null;
            return {
              conversations: state.conversations.map((conversation) =>
                conversation.id === event.conversationId
                  ? { ...conversation, title: event.title }
                  : conversation,
              ),
              tabsById: Object.fromEntries(
                Object.entries(state.tabsById).map(([tabId, tab]) => [
                  tabId,
                  tab.conversationId === event.conversationId
                    ? { ...tab, title: event.title }
                    : tab,
                ]),
              ),
              settlingTitles:
                previous !== null && previous !== event.title
                  ? { ...state.settlingTitles, [event.conversationId]: previous }
                  : state.settlingTitles,
            };
          });
          window.setTimeout(() => {
            set((state) => {
              if (!(event.conversationId in state.settlingTitles)) return state;
              const settlingTitles = { ...state.settlingTitles };
              delete settlingTitles[event.conversationId];
              return { settlingTitles };
            });
          }, TITLE_SETTLE_MS);
        }),
      ]);
      unlisteners = [
        stopModels,
        stopPreferences,
        stopCompanions,
        stopWokenEvents,
        stopSlept,
        stopTitled,
      ];
      set({ isInitialised: true, isInitialising: false });
    } catch (error) {
      const tab = newConversationTab();
      tab.error = errorMessage(error);
      set({
        isInitialised: true,
        isInitialising: false,
        activeTabId: tab.id,
        tabOrder: [tab.id],
        tabsById: { [tab.id]: tab },
      });
    }
  },

  dispose: () => {
    unlisteners.forEach((unlisten) => unlisten());
    unlisteners = [];
    set({ isInitialised: false, isInitialising: false });
  },

  // "Chat" from the sidebar means a conversation, not whichever tab is up:
  // if the settings tab is the active one, step to the nearest conversation
  // tab (or open one) so the click always lands on a thread.
  setActiveView: (view) => {
    const state = get();
    const active = state.activeTabId ? state.tabsById[state.activeTabId] : null;
    if (view !== "chat" || !active || active.kind === "conversation") {
      set({ activeView: view });
      return;
    }
    const index = state.tabOrder.indexOf(active.id);
    const nearest =
      state.tabOrder
        .map((id, at) => ({ id, distance: Math.abs(at - index) }))
        .filter(({ id }) => state.tabsById[id]?.kind === "conversation")
        .sort((left, right) => left.distance - right.distance)[0] ?? null;
    if (nearest) get().setActiveTab(nearest.id);
    else get().openNewConversation();
  },

  openSettings: () => {
    if (get().tabsById[SETTINGS_TAB_ID]) {
      get().setActiveTab(SETTINGS_TAB_ID);
      return;
    }
    const tab = settingsTab();
    set((state) => ({
      activeView: "chat",
      activeTabId: tab.id,
      tabOrder: [...state.tabOrder, tab.id],
      tabsById: { ...state.tabsById, [tab.id]: tab },
    }));
  },

  openConversation: async (conversationId) => {
    const existing = Object.values(get().tabsById).find(
      (tab) => tab.conversationId === conversationId,
    );
    if (existing) {
      get().setActiveTab(existing.id);
      return;
    }

    const conversation = get().conversations.find((item) => item.id === conversationId);
    if (!conversation) return;
    const tab = tabForConversation(conversation);
    const existingRuntime = get().runtimeByConversationId[conversationId];
    set((state) => ({
      activeView: "chat",
      activeTabId: tab.id,
      tabOrder: [...state.tabOrder, tab.id],
      tabsById: { ...state.tabsById, [tab.id]: tab },
      runtimeByConversationId: {
        ...state.runtimeByConversationId,
        [conversationId]: existingRuntime ?? emptyRuntime(true),
      },
    }));

    if (existingRuntime && !existingRuntime.isLoading) {
      requestConversationScrollToEnd(conversationId);
      return;
    }

    try {
      const thread = await getConversationThread(conversationId);
      set((state) => {
        if (!state.tabsById[tab.id]) return state;
        return {
          conversations: reconcileConversation(state.conversations, thread.conversation),
          tabsById: {
            ...state.tabsById,
            [tab.id]: {
              ...state.tabsById[tab.id],
              title: thread.conversation.title,
              companionId: thread.conversation.companionId,
            },
          },
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [conversationId]: {
              messages: thread.messages,
              isLoading: false,
              isStreaming: thread.messages.some((message) => message.status === "streaming"),
              isRemembering:
                state.runtimeByConversationId[conversationId]?.isRemembering ?? false,
              error: null,
              recallByMessageId:
                state.runtimeByConversationId[conversationId]?.recallByMessageId ?? {},
              toolCallsByMessageId:
                state.runtimeByConversationId[conversationId]?.toolCallsByMessageId ?? {},
              reasoningByMessageId:
                state.runtimeByConversationId[conversationId]?.reasoningByMessageId ?? {},
              thinkingMessageId:
                state.runtimeByConversationId[conversationId]?.thinkingMessageId ?? null,
            },
          },
        };
      });
      requestConversationScrollToEnd(conversationId);
    } catch (error) {
      set((state) => ({
        runtimeByConversationId: {
          ...state.runtimeByConversationId,
          [conversationId]: {
            ...(state.runtimeByConversationId[conversationId] ?? emptyRuntime()),
            isLoading: false,
            error: errorMessage(error),
          },
        },
      }));
    }
  },

  openNewConversation: () => {
    // The last companion picked is the next one offered (s571: "it always
    // defaulting to Rook"). Read through the roster, so a remembered pick
    // that names nobody any more falls back to the built-in companion
    // rather than opening a tab on a face that no longer exists.
    const { userPreferences, companions } = get();
    const remembered = userPreferences.defaultCompanionId;
    const tab = newConversationTab(
      remembered && companions.some((companion) => companion.id === remembered)
        ? remembered
        : null,
    );
    set((state) => ({
      activeView: "chat",
      activeTabId: tab.id,
      tabOrder: [...state.tabOrder, tab.id],
      tabsById: { ...state.tabsById, [tab.id]: tab },
    }));
  },

  setActiveTab: (tabId) => {
    if (!get().tabsById[tabId]) return;
    set((state) => ({
      activeView: "chat",
      activeTabId: tabId,
      tabsById: {
        ...state.tabsById,
        [tabId]: { ...state.tabsById[tabId], unreadCount: 0 },
      },
    }));
  },

  closeTab: (tabId) => {
    const state = get();
    const index = state.tabOrder.indexOf(tabId);
    if (index < 0) return;
    const nextOrder = state.tabOrder.filter((id) => id !== tabId);
    const nextTabs = { ...state.tabsById };
    delete nextTabs[tabId];
    const nextSubmitting = { ...state.submittingByTabId };
    delete nextSubmitting[tabId];
    const nextActiveId =
      state.activeTabId === tabId
        ? (nextOrder[Math.min(index, nextOrder.length - 1)] ?? null)
        : state.activeTabId;
    set({
      tabOrder: nextOrder,
      tabsById: nextTabs,
      submittingByTabId: nextSubmitting,
      activeTabId: nextActiveId,
    });
    if (nextOrder.length === 0) get().openNewConversation();
  },

  addAttachments: (tabId, attachments) => {
    set((state) => {
      const tab = state.tabsById[tabId];
      if (!tab || attachments.length === 0) return state;
      const merged = [...tab.attachments, ...attachments].slice(
        0,
        MAX_ATTACHMENTS_PER_MESSAGE,
      );
      const dropped = tab.attachments.length + attachments.length - merged.length;
      return {
        tabsById: {
          ...state.tabsById,
          [tabId]: {
            ...tab,
            attachments: merged,
            error: null,
            notice:
              dropped > 0
                ? `A message can carry up to ${MAX_ATTACHMENTS_PER_MESSAGE} images — ${dropped} left out.`
                : tab.notice,
          },
        },
      };
    });
  },

  attachFiles: async (tabId, files) => {
    if (files.length === 0) return;
    try {
      const prepared = await Promise.all(
        files.map((file) => prepareImageAttachment(file)),
      );
      get().addAttachments(tabId, prepared);
    } catch (error) {
      set((state) => {
        const tab = state.tabsById[tabId];
        return tab
          ? {
              tabsById: {
                ...state.tabsById,
                [tabId]: { ...tab, error: errorMessage(error) },
              },
            }
          : state;
      });
    }
  },

  removeAttachment: (tabId, attachmentId) => {
    set((state) => {
      const tab = state.tabsById[tabId];
      if (!tab) return state;
      return {
        tabsById: {
          ...state.tabsById,
          [tabId]: {
            ...tab,
            attachments: tab.attachments.filter(
              (attachment) => attachment.id !== attachmentId,
            ),
          },
        },
      };
    });
  },

  setDraft: (tabId, draft) => {
    if (!get().tabsById[tabId]) return;
    set((state) => ({
      tabsById: {
        ...state.tabsById,
        [tabId]: { ...state.tabsById[tabId], draft },
      },
    }));
  },

  setTabCompanion: async (tabId, companionId) => {
    const tab = get().tabsById[tabId];
    if (!tab || tab.companionId === companionId) return;
    const previous = tab.companionId;
    // The pick is also the person's standing preference: the next new
    // conversation opens on it. Fail-open — the tab already changed, and a
    // preference that did not save is a convenience lost, not an error to
    // put in the composer.
    const remember = () => {
      void updateUserPreferences({ defaultCompanionId: companionId })
        .then((userPreferences) => set({ userPreferences }))
        .catch(() => undefined);
    };
    // A tab with no conversation yet has nothing to persist against; the pick
    // rides along on the first send instead.
    if (!tab.conversationId) {
      set((state) => ({
        tabsById: {
          ...state.tabsById,
          [tabId]: { ...state.tabsById[tabId], companionId, error: null },
        },
      }));
      remember();
      return;
    }
    // A thread with a conversation behind it has been spoken in, and Rust
    // will refuse to re-point it (s569). No optimistic flip here: the picker
    // is closed on such a tab, so reaching this is a stale control — let the
    // refusal land as the tab's error instead of flashing a change that
    // never happened.

    try {
      const conversation = await updateConversationCompanion({
        conversationId: tab.conversationId,
        companionId,
      });
      set((state) => ({
        conversations: reconcileConversation(state.conversations, conversation),
        tabsById: Object.fromEntries(
          Object.entries(state.tabsById).map(([id, candidate]) => [
            id,
            candidate.conversationId === conversation.id
              ? { ...candidate, companionId: conversation.companionId }
              : candidate,
          ]),
        ),
      }));
      remember();
    } catch (error) {
      set((state) => ({
        tabsById: {
          ...state.tabsById,
          [tabId]: {
            ...state.tabsById[tabId],
            companionId: previous,
            error: errorMessage(error),
          },
        },
      }));
    }
  },

  setUserDefaultModel: async (preference) => {
    set({ preferenceError: null });
    try {
      const userPreferences = await updateUserPreferences({ defaultModel: preference });
      set({ userPreferences, preferenceError: null });
    } catch (error) {
      set({ preferenceError: errorMessage(error) });
    }
  },

  // Only the name is sent — the update is a patch, so the default model is not
  // re-asserted from this screen's copy of it and cannot be reverted by a save
  // made here.
  setUserDisplayName: async (name) => {
    set({ preferenceError: null });
    try {
      const userPreferences = await updateUserPreferences({ displayName: name });
      set({ userPreferences, preferenceError: null });
    } catch (error) {
      set({ preferenceError: errorMessage(error) });
      throw error;
    }
  },

  sendMessage: async (tabId, content) => {
    const tab = get().tabsById[tabId];
    const message = content.trim();
    const attachments = tab?.attachments ?? [];
    if (!tab || (!message && attachments.length === 0) || get().submittingByTabId[tabId])
      return;
    const runtime = tab.conversationId
      ? get().runtimeByConversationId[tab.conversationId]
      : undefined;
    if (runtime?.isStreaming) return;

    if (message === "/sleep") {
      await get().sleepActiveConversation(tabId);
      return;
    }

    set((state) => ({
      submittingByTabId: { ...state.submittingByTabId, [tabId]: true },
      tabsById: {
        ...state.tabsById,
        [tabId]: {
          ...state.tabsById[tabId],
          draft: "",
          attachments: [],
          error: null,
          notice: null,
        },
      },
    }));

    // The optimistic echo: the message is on screen the moment Enter lands.
    // The memory pass and the submit round-trip still run before the backend
    // accepts — but the user should never watch that silence. The echo is
    // replaced by the authoritative row on accept, and withdrawn (back into
    // the composer, same as always) if the send fails. A brand-new
    // conversation has no runtime to echo into until accept names it — that
    // first send keeps the old timing.
    const conversationId = tab.conversationId;
    const optimisticId = conversationId ? `optimistic-${crypto.randomUUID()}` : null;
    if (conversationId && optimisticId) {
      set((state) => {
        const runtimeState = state.runtimeByConversationId[conversationId];
        if (!runtimeState) return state;
        const now = Date.now();
        return {
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [conversationId]: {
              ...runtimeState,
              messages: [
                ...runtimeState.messages,
                {
                  id: optimisticId,
                  conversationId,
                  sequence:
                    runtimeState.messages.reduce(
                      (max, item) => Math.max(max, item.sequence),
                      0,
                    ) + 1,
                  role: "user",
                  status: "pending",
                  content: message,
                  providerId: null,
                  modelId: null,
                  companionId: tab.companionId,
                  errorMessage: null,
                  createdAt: now,
                  updatedAt: now,
                  completedAt: null,
                  attachments,
                },
              ],
            },
          },
        };
      });
      requestConversationScrollToEnd(conversationId);
    }

    // Memory rides ahead of the message — fail-open, never blocks the send.
    // The sleeper's consent rides beside it: which brain may sleep on this
    // thread once the turn lands (null = leave it for a manual /sleep).
    const [memory, autoSleepAgentId] = await Promise.all([
      runMemoryPreSend({
        conversationId: tab.conversationId,
        companionId: tab.companionId,
        text: message,
        messages: runtime?.messages ?? [],
      }),
      autoSleepAgent(tab.companionId),
    ]);

    if (stopRequests.has(tabId)) {
      // Stopped before Rust ever saw it: the words go back into the
      // composer, the echo comes down, and nothing was sent.
      stopRequests.delete(tabId);
      set((state) => {
        const currentTab = state.tabsById[tabId];
        const runtimeState = conversationId
          ? state.runtimeByConversationId[conversationId]
          : undefined;
        const submittingByTabId = { ...state.submittingByTabId };
        delete submittingByTabId[tabId];
        return {
          submittingByTabId,
          runtimeByConversationId:
            conversationId && runtimeState && optimisticId
              ? {
                  ...state.runtimeByConversationId,
                  [conversationId]: {
                    ...runtimeState,
                    messages: runtimeState.messages.filter(
                      (item) => item.id !== optimisticId,
                    ),
                  },
                }
              : state.runtimeByConversationId,
          tabsById: currentTab
            ? {
                ...state.tabsById,
                [tabId]: { ...currentTab, draft: message, attachments, notice: "Stopped." },
              }
            : state.tabsById,
        };
      });
      return;
    }

    let wasAccepted = false;
    // One event folded into the store — pure, so a frame's worth of deltas
    // can be applied in a single set (below) instead of one commit each.
    const reduceChatEvent = (
      state: CompanionStore,
      event: Exclude<ChatEvent, { kind: "callSpeechDelta" | "callSpeechFinished" }>,
    ): Partial<CompanionStore> => {
      if (event.kind === "accepted") {
        const conversationId = event.conversation.id;
        const currentTab = state.tabsById[tabId];
        const runtimeState =
          state.runtimeByConversationId[conversationId] ?? emptyRuntime();
        return {
          conversations: reconcileConversation(state.conversations, event.conversation),
          tabsById: currentTab
            ? {
                ...state.tabsById,
                [tabId]: {
                  ...currentTab,
                  conversationId,
                  title: event.conversation.title,
                  companionId: event.conversation.companionId,
                },
              }
            : state.tabsById,
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [conversationId]: {
              ...runtimeState,
              // The authoritative row takes the optimistic echo's place.
              messages: reconcileMessage(
                optimisticId
                  ? runtimeState.messages.filter((item) => item.id !== optimisticId)
                  : runtimeState.messages,
                event.message,
              ),
              // The 🧠 chip pins to the accepted user message — the memory
              // pass already ran for this send by the time we get an id.
              recallByMessageId: memory
                ? {
                    ...runtimeState.recallByMessageId,
                    [event.message.id]: memory.chip,
                  }
                : runtimeState.recallByMessageId,
              error: null,
            },
          },
        };
      }

      const conversationId =
        event.kind === "assistantStarted" || event.kind === "assistantCompleted"
          ? event.message.conversationId
          : event.conversationId;
      const runtimeState =
        state.runtimeByConversationId[conversationId] ?? emptyRuntime();
      if (event.kind === "toolCall") {
        return {
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [conversationId]: {
              ...runtimeState,
              // A tool signal means the model stopped thinking and asked
              // for something. If it thinks again after the result, the
              // next reasoning delta re-arms this.
              thinkingMessageId: null,
              toolCallsByMessageId: {
                ...runtimeState.toolCallsByMessageId,
                [event.messageId]: reconcileToolCall(
                  runtimeState.toolCallsByMessageId[event.messageId] ?? [],
                  {
                    callId: event.callId,
                    name: event.name,
                    arguments: event.arguments,
                    status: event.status,
                    detail: event.detail,
                    afterText: event.afterText,
                  },
                  Date.now(),
                ),
              },
            },
          },
        };
      }
      if (event.kind === "remembering") {
        return {
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [conversationId]: { ...runtimeState, isRemembering: event.active },
          },
        };
      }
      if (event.kind === "assistantStarted") {
        return {
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [conversationId]: {
              ...runtimeState,
              messages: reconcileMessage(runtimeState.messages, event.message),
              isStreaming: true,
              error: null,
            },
          },
        };
      }
      if (event.kind === "assistantDelta") {
        return {
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [conversationId]: {
              ...runtimeState,
              isStreaming: true,
              // The first word of the answer ends the thinking — whichever
              // row the thoughts landed on (after a tool, the text opens a
              // new row while the thoughts stayed on the old one).
              thinkingMessageId: null,
              messages: runtimeState.messages.map((item) =>
                item.id === event.messageId
                  ? {
                      ...item,
                      content: item.content + event.delta,
                      status: "streaming",
                      updatedAt: Date.now(),
                    }
                  : item,
              ),
            },
          },
        };
      }
      if (event.kind === "assistantReasoningDelta") {
        return {
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [conversationId]: {
              ...runtimeState,
              isStreaming: true,
              thinkingMessageId: event.messageId,
              reasoningByMessageId: {
                ...runtimeState.reasoningByMessageId,
                [event.messageId]:
                  (runtimeState.reasoningByMessageId[event.messageId] ?? "") + event.delta,
              },
            },
          },
        };
      }
      if (event.kind === "assistantCompleted") {
        const targetTab = Object.values(state.tabsById).find(
          (candidate) => candidate.conversationId === conversationId,
        );
        const isVisible =
          state.activeView === "chat" && targetTab?.id === state.activeTabId;
        return {
          tabsById: targetTab
            ? {
                ...state.tabsById,
                [targetTab.id]: {
                  ...targetTab,
                  unreadCount: isVisible ? 0 : targetTab.unreadCount + 1,
                },
              }
            : state.tabsById,
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [conversationId]: {
              ...runtimeState,
              messages: reconcileMessage(runtimeState.messages, event.message),
              isStreaming: false,
              isRemembering: false,
              thinkingMessageId: null,
              error: null,
            },
          },
        };
      }

      return {
        runtimeByConversationId: {
          ...state.runtimeByConversationId,
          [conversationId]: {
            ...runtimeState,
            isStreaming: false,
            isRemembering: false,
            thinkingMessageId: null,
            error: event.message,
            messages: event.messageId
              ? runtimeState.messages.map((item) =>
                  item.id === event.messageId
                    ? { ...item, status: "failed", errorMessage: event.message }
                    : item,
                )
              : runtimeState.messages,
          },
        },
      };
    };

    // ⚑ THE STREAM IS THE APP'S MOST PERF-SENSITIVE PATH (Studio, s335). Rust
    // emits one event per token and every store commit re-renders the shell,
    // so a fast local model produced a full-tree render — with the thread's
    // forced layout — dozens of times a second, and the window stopped
    // answering. Deltas are queued here and folded into ONE commit per
    // animation frame; what the reader sees is the same text at the same
    // moment, arriving in one write instead of many. Every other kind lands
    // at once, behind whatever deltas came before it, so order is kept.
    let pendingDeltas: StreamDelta[] = [];
    let flushFrame: number | null = null;
    const flushDeltas = () => {
      if (flushFrame !== null) {
        window.cancelAnimationFrame(flushFrame);
        flushFrame = null;
      }
      if (pendingDeltas.length === 0) return;
      const batch = foldStreamDeltas(pendingDeltas);
      pendingDeltas = [];
      set((state) => {
        let next = state;
        for (const delta of batch) next = { ...next, ...reduceChatEvent(next, delta) };
        return next;
      });
    };
    const handleChatEvent = (event: ChatEvent) => {
      // Call cards own their transient speech through the app-wide event bus;
      // keep this boundary explicit if another sink forwards those variants.
      if (event.kind === "callSpeechDelta" || event.kind === "callSpeechFinished") return;
      if (isStreamDelta(event)) {
        pendingDeltas.push(event);
        if (flushFrame === null) flushFrame = window.requestAnimationFrame(flushDeltas);
        return;
      }
      flushDeltas();
      if (event.kind === "accepted") wasAccepted = true;
      set((state) => reduceChatEvent(state, event));
      requestScrollForChatEvent(event);
    };

    try {
      const accepted = await submitMessage(
        {
          conversationId: tab.conversationId,
          companionId: tab.companionId,
          content: message,
          memoryContext: memory?.injection || null,
          memoryAgentId: memory?.agentId ?? null,
          autoSleepAgentId,
          attachments: attachments.map((attachment) => ({
            mediaType: attachment.mediaType,
            data: attachment.data,
          })),
        },
        handleChatEvent,
      );
      handleChatEvent(acceptedEvent(accepted));
    } catch (error) {
      set((state) => {
        const currentTab = state.tabsById[tabId];
        // A failed send goes back into the composer, so the optimistic echo
        // must not stay behind as a ghost row. After accept the filter is a
        // no-op — the echo is already gone.
        const runtimeState = conversationId
          ? state.runtimeByConversationId[conversationId]
          : undefined;
        const runtimeByConversationId =
          conversationId && runtimeState && optimisticId
            ? {
                ...state.runtimeByConversationId,
                [conversationId]: {
                  ...runtimeState,
                  messages: runtimeState.messages.filter(
                    (item) => item.id !== optimisticId,
                  ),
                },
              }
            : state.runtimeByConversationId;
        return currentTab
          ? {
              runtimeByConversationId,
              tabsById: {
                ...state.tabsById,
                [tabId]: {
                  ...currentTab,
                  draft: wasAccepted ? currentTab.draft : message,
                  attachments: wasAccepted ? currentTab.attachments : attachments,
                  error: errorMessage(error),
                },
              },
            }
          : { runtimeByConversationId };
      });
    } finally {
      stopRequests.delete(tabId);
      // Whatever the last frame had not shown yet lands before the settle,
      // so no delta can arrive after the row is marked done.
      flushDeltas();
      set((state) => {
        const submittingByTabId = { ...state.submittingByTabId };
        delete submittingByTabId[tabId];
        // A brand-new conversation only learns its id on accept — read the
        // tab, not the id this send started with.
        const settledId = state.tabsById[tabId]?.conversationId ?? conversationId;
        const runtimeState = settledId ? state.runtimeByConversationId[settledId] : undefined;
        if (!settledId || !runtimeState) return { submittingByTabId };
        return {
          submittingByTabId,
          runtimeByConversationId: {
            ...state.runtimeByConversationId,
            [settledId]: settleRuntime(runtimeState, Date.now()),
          },
        };
      });
    }
  },

  stopTurn: async (tabId) => {
    const tab = get().tabsById[tabId];
    if (!tab) return;
    const runtime = tab.conversationId
      ? get().runtimeByConversationId[tab.conversationId]
      : undefined;
    if (!get().submittingByTabId[tabId] && !runtime?.isStreaming) return;
    // The send may not have reached Rust yet (the memory pass runs first):
    // note the wish, and sendMessage honours it before submitting.
    stopRequests.add(tabId);
    if (!tab.conversationId) return;
    try {
      const stopped = await requestStopTurn(tab.conversationId);
      if (!stopped) return;
      set((state) => {
        const currentTab = state.tabsById[tabId];
        return currentTab
          ? { tabsById: { ...state.tabsById, [tabId]: { ...currentTab, notice: "Stopped." } } }
          : state;
      });
    } catch (error) {
      set((state) => {
        const currentTab = state.tabsById[tabId];
        return currentTab
          ? {
              tabsById: {
                ...state.tabsById,
                [tabId]: { ...currentTab, error: errorMessage(error) },
              },
            }
          : state;
      });
    }
  },

  sleepActiveConversation: async (tabId) => {
    const tab = get().tabsById[tabId];
    if (!tab) return;
    if (!tab.conversationId) {
      set((state) => ({
        tabsById: {
          ...state.tabsById,
          [tabId]: {
            ...state.tabsById[tabId],
            error: "There is nothing to sleep yet — say something first.",
          },
        },
      }));
      return;
    }

    const conversationId = tab.conversationId;
    set((state) => ({
      submittingByTabId: { ...state.submittingByTabId, [tabId]: true },
      tabsById: {
        ...state.tabsById,
        [tabId]: {
          ...state.tabsById[tabId],
          draft: "",
          error: null,
          notice: "Sleeping — distilling this conversation into memory…",
        },
      },
    }));

    // The live pulse: one notification that morphs through the stages while
    // the pass runs. The composer notice stays the in-place record.
    const { notify, updateNotification } = useNotificationsStore.getState();
    const notificationId = notify({
      title: "💤 Sleep",
      text: "Preparing the conversation…",
      status: "active",
    });

    try {
      const outcome = await sleepConversation(conversationId, tab.companionId, (event) => {
        if (event.type === "stage" && event.stage === "distilling") {
          updateNotification(notificationId, {
            text: `Distilling ${event.turns} new turns — the model is reading the conversation…`,
          });
        } else if (event.type === "stage" && event.stage === "carving") {
          updateNotification(notificationId, {
            text:
              event.total === 0
                ? "Nothing durable to carve from this conversation."
                : `Carving ${event.total} memories…`,
            progress: event.total > 0 ? { done: 0, total: event.total } : null,
          });
        } else if (event.type === "carved") {
          updateNotification(notificationId, {
            text: `Carving ${event.done}/${event.total} — ${event.name}`,
            progress: { done: event.done, total: event.total },
          });
        }
      });

      if (outcome.nothingNew) {
        const message =
          "Nothing new to sleep on — every turn here is already remembered.";
        updateNotification(notificationId, {
          status: "info",
          text: message,
          progress: null,
        });
        set((state) => ({
          tabsById: {
            ...state.tabsById,
            [tabId]: { ...state.tabsById[tabId], notice: message },
          },
        }));
        return;
      }

      const carved =
        outcome.memories.length > 0 ? ` — ${outcome.memories.join(", ")}` : "";
      // A borrowed scribe is stated, never silent: the memories are this
      // companion's, but another model's hand wrote them.
      const scribe = outcome.scribeNote ? ` · ${outcome.scribeNote}` : "";
      const summary =
        `Slept: ${outcome.created} carved, ${outcome.updated} updated` +
        `${outcome.dropped ? `, ${outcome.dropped} dropped` : ""}`;
      updateNotification(notificationId, {
        status: "success",
        text: summary,
        progress: null,
      });
      set((state) => ({
        tabsById: {
          ...state.tabsById,
          [tabId]: {
            ...state.tabsById[tabId],
            notice: `${summary}${carved}${scribe}`,
          },
        },
      }));
    } catch (error) {
      updateNotification(notificationId, {
        status: "error",
        text: errorMessage(error),
        progress: null,
      });
      set((state) => ({
        tabsById: {
          ...state.tabsById,
          [tabId]: {
            ...state.tabsById[tabId],
            notice: null,
            error: errorMessage(error),
          },
        },
      }));
    } finally {
      set((state) => {
        const submittingByTabId = { ...state.submittingByTabId };
        delete submittingByTabId[tabId];
        return { submittingByTabId };
      });
    }
  },
}));
