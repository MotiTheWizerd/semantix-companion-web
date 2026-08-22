import type { UnlistenFn } from "@tauri-apps/api/event";
import { create } from "zustand";

import {
  getConversationThread,
  listConversations,
  submitMessage,
  updateConversationModelPreference,
} from "../chat/chatService";
import { requestConversationScrollToEnd } from "../chat/chatScrollEvents";
import type {
  AcceptedMessage,
  ChatEvent,
  ChatMessage,
  Conversation,
} from "../chat/types";
import {
  listConfiguredModels,
  onModelsChanged,
} from "../models/configuredModels/modelService";
import type { ConfiguredModel } from "../models/configuredModels/types";
import { runMemoryPreSend, type MemoryRecallChipData } from "../memory/preSend";
import { sleepConversation } from "../memory/sleepService";
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
  modelPreference: ModelPreference;
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
}

interface CompanionStore {
  activeView: WorkspaceView;
  isInitialising: boolean;
  isInitialised: boolean;
  conversations: Conversation[];
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
  setTabModelPreference: (tabId: string, preference: ModelPreference) => Promise<void>;
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
    modelPreference: { mode: "inherit" },
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
    modelPreference: conversation.modelPreference,
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

function requestScrollForChatEvent(event: ChatEvent): void {
  if (event.kind === "accepted") {
    requestConversationScrollToEnd(event.conversation.id);
  } else if (event.kind === "assistantDelta") {
    requestConversationScrollToEnd(event.conversationId);
  } else if (event.kind === "assistantStarted" || event.kind === "assistantCompleted") {
    requestConversationScrollToEnd(event.message.conversationId);
  }
}

function modelPreferencesEqual(left: ModelPreference, right: ModelPreference): boolean {
  return (
    left.mode === right.mode &&
    (left.mode !== "configured" ||
      (right.mode === "configured" && left.modelId === right.modelId))
  );
}

export const useCompanionStore = create<CompanionStore>()((set, get) => ({
  activeView: "chat",
  isInitialising: false,
  isInitialised: false,
  conversations: [],
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
      const [conversations, configuredModels, userPreferences] = await Promise.all([
        listConversations(),
        listConfiguredModels(),
        getUserPreferences(),
      ]);
      set({ conversations, configuredModels, userPreferences });

      if (conversations[0]) {
        await get().openConversation(conversations[0].id);
      } else {
        get().openNewConversation();
      }

      const [stopModels, stopPreferences] = await Promise.all([
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
                          modelPreference: conversation.modelPreference,
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
      ]);
      unlisteners = [stopModels, stopPreferences];
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
              modelPreference: thread.conversation.modelPreference,
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

  setDraft: (tabId, draft) => {
    if (!get().tabsById[tabId]) return;
    set((state) => ({
      tabsById: {
        ...state.tabsById,
        [tabId]: { ...state.tabsById[tabId], draft },
      },
    }));
  },

  setTabModelPreference: async (tabId, preference) => {
    const tab = get().tabsById[tabId];
    if (!tab || modelPreferencesEqual(tab.modelPreference, preference)) return;
    const previous = tab.modelPreference;
    set((state) => ({
      tabsById: {
        ...state.tabsById,
        [tabId]: { ...state.tabsById[tabId], modelPreference: preference, error: null },
      },
    }));
    if (!tab.conversationId) return;

    try {
      const conversation = await updateConversationModelPreference({
        conversationId: tab.conversationId,
        modelPreference: preference,
      });
      set((state) => ({
        conversations: reconcileConversation(state.conversations, conversation),
        tabsById: Object.fromEntries(
          Object.entries(state.tabsById).map(([id, candidate]) => [
            id,
            candidate.conversationId === conversation.id
              ? { ...candidate, modelPreference: conversation.modelPreference }
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
            modelPreference: previous,
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
    if (!tab || !message || get().submittingByTabId[tabId]) return;
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
        [tabId]: { ...state.tabsById[tabId], draft: "", error: null, notice: null },
      },
    }));

    // Memory rides ahead of the message — fail-open, never blocks the send.
    const memory = await runMemoryPreSend({
      conversationId: tab.conversationId,
      text: message,
      messages: runtime?.messages ?? [],
    });

    let wasAccepted = false;
    const handleChatEvent = (event: ChatEvent) => {
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
                    modelPreference: event.conversation.modelPreference,
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
          modelPreference: tab.modelPreference,
          content: message,
          memoryContext: memory?.injection || null,
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

    try {
      const outcome = await sleepConversation(conversationId);
      const carved =
        outcome.memories.length > 0 ? ` — ${outcome.memories.join(", ")}` : "";
      set((state) => ({
        tabsById: {
          ...state.tabsById,
          [tabId]: {
            ...state.tabsById[tabId],
            notice:
              `Slept: ${outcome.created} carved, ${outcome.updated} updated` +
              `${outcome.dropped ? `, ${outcome.dropped} dropped` : ""}${carved}`,
          },
        },
      }));
    } catch (error) {
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

export function effectiveModelPreference(
  preference: ModelPreference,
  userPreferences: UserPreferences,
): Exclude<ModelPreference, { mode: "inherit" }> {
  return preference.mode === "inherit" ? userPreferences.defaultModel : preference;
}
