import type { UnlistenFn } from "@tauri-apps/api/event";
import { create } from "zustand";

import {
  getConversationThread,
  listConversations,
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
import { sleepConversation } from "../memory/sleepService";
import { useNotificationsStore } from "../notifications/notificationsStore";
import {
  getUserPreferences,
  onUserPreferencesChanged,
  updateUserPreferences,
} from "../preferences/preferenceService";
import type { ModelPreference, UserPreferences } from "../preferences/types";

export type WorkspaceView = "chat" | "settings";

export interface ConversationTab {
  id: string;
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
  error: string | null;
  /** 🧠 chip per sent user message — live-session instrument, not persisted,
   *  so it dies with the runtime entry (reload = no chips, by design). */
  recallByMessageId: Record<string, MemoryRecallChipData>;
  /** 📖 tool chips per assistant message — same live-session contract. */
  toolCallsByMessageId: Record<string, ToolCallChipItem[]>;
  /** Provider-supplied thoughts + tool-round progress narration per assistant
   *  message. Runtime-only so neither becomes later conversation context. */
  reasoningByMessageId: Record<string, string>;
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
  initialise: () => Promise<void>;
  dispose: () => void;
  setActiveView: (view: WorkspaceView) => void;
  openConversation: (conversationId: string) => Promise<void>;
  openNewConversation: () => void;
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
  sendMessage: (tabId: string, content: string) => Promise<void>;
  /** The /sleep pass: distill the tab's conversation into long-term memory. */
  sleepActiveConversation: (tabId: string) => Promise<void>;
}

const EMPTY_USER_PREFERENCES: UserPreferences = {
  defaultModel: { mode: "test" },
  updatedAt: 0,
};

let unlisteners: UnlistenFn[] = [];

function createTabId(prefix: "new" | "conversation", id?: string): string {
  return id ? `${prefix}:${id}` : `${prefix}:${crypto.randomUUID()}`;
}

function newConversationTab(): ConversationTab {
  return {
    id: createTabId("new"),
    conversationId: null,
    title: "New conversation",
    draft: "",
    attachments: [],
    companionId: null,
    unreadCount: 0,
    error: null,
    notice: null,
  };
}

function tabForConversation(conversation: Conversation): ConversationTab {
  return {
    id: createTabId("conversation", conversation.id),
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

function emptyRuntime(isLoading = false): ConversationRuntime {
  return {
    messages: [],
    isLoading,
    isStreaming: false,
    error: null,
    recallByMessageId: {},
    toolCallsByMessageId: {},
    reasoningByMessageId: {},
  };
}

/** Upsert one tool-call lifecycle event into a message's chip list —
 *  "running" appends, "ok"/"error" replaces the running entry in place. */
function reconcileToolCall(
  calls: ToolCallChipItem[],
  event: ToolCallChipItem,
): ToolCallChipItem[] {
  const index = calls.findIndex((call) => call.callId === event.callId);
  if (index < 0) return [...calls, event];
  return calls.map((call, at) => (at === index ? event : call));
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

function requestScrollForChatEvent(event: ChatEvent): void {
  if (event.kind === "accepted") {
    requestConversationScrollToEnd(event.conversation.id);
  } else if (
    event.kind === "assistantDelta" ||
    event.kind === "assistantContentReplaced" ||
    event.kind === "assistantReasoningDelta" ||
    event.kind === "toolCall"
  ) {
    requestConversationScrollToEnd(event.conversationId);
  } else if (event.kind === "assistantStarted" || event.kind === "assistantCompleted") {
    requestConversationScrollToEnd(event.message.conversationId);
  }
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

      const [stopModels, stopPreferences, stopCompanions] = await Promise.all([
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
      ]);
      unlisteners = [stopModels, stopPreferences, stopCompanions];
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

  setActiveView: (view) => set({ activeView: view }),

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
              error: null,
              recallByMessageId:
                state.runtimeByConversationId[conversationId]?.recallByMessageId ?? {},
              toolCallsByMessageId:
                state.runtimeByConversationId[conversationId]?.toolCallsByMessageId ?? {},
              reasoningByMessageId:
                state.runtimeByConversationId[conversationId]?.reasoningByMessageId ?? {},
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
    const tab = newConversationTab();
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
    set((state) => ({
      tabsById: {
        ...state.tabsById,
        [tabId]: { ...state.tabsById[tabId], companionId, error: null },
      },
    }));
    // A tab with no conversation yet has nothing to persist against; the pick
    // rides along on the first send instead.
    if (!tab.conversationId) return;

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

    // Memory rides ahead of the message — fail-open, never blocks the send.
    const memory = await runMemoryPreSend({
      conversationId: tab.conversationId,
      companionId: tab.companionId,
      text: message,
      messages: runtime?.messages ?? [],
    });

    let wasAccepted = false;
    const handleChatEvent = (event: ChatEvent) => {
      // Call cards own their transient speech through the app-wide event bus;
      // keep this boundary explicit if another sink forwards those variants.
      if (event.kind === "callSpeechDelta" || event.kind === "callSpeechFinished") return;
      if (event.kind === "accepted") wasAccepted = true;
      set((state) => {
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
                messages: reconcileMessage(runtimeState.messages, event.message),
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
                    },
                  ),
                },
              },
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
        if (event.kind === "assistantContentReplaced") {
          return {
            runtimeByConversationId: {
              ...state.runtimeByConversationId,
              [conversationId]: {
                ...runtimeState,
                isStreaming: true,
                messages: runtimeState.messages.map((item) =>
                  item.id === event.messageId
                    ? {
                        ...item,
                        content: event.content,
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
      });
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
        return currentTab
          ? {
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
          : state;
      });
    } finally {
      set((state) => {
        const submittingByTabId = { ...state.submittingByTabId };
        delete submittingByTabId[tabId];
        return { submittingByTabId };
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
