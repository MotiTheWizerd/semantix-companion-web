import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";

import {
  listConfiguredModels,
  onModelsChanged,
} from "../models/configuredModels/modelService";
import type { ConfiguredModel } from "../models/configuredModels/types";
import { getConversationThread, listConversations, submitMessage } from "./chatService";
import { requestConversationScrollToEnd } from "./chatScrollEvents";
import type {
  AcceptedMessage,
  ChatEvent,
  ChatMessage,
  Conversation,
  ConversationThread,
} from "./types";

interface ChatControllerState {
  conversations: Conversation[];
  configuredModels: ConfiguredModel[];
  selectedModelId: string | null;
  activeConversationId: string | null;
  messages: ChatMessage[];
  isLoading: boolean;
  isSending: boolean;
  error: string | null;
}

type ChatAction =
  | {
      type: "initialised";
      conversations: Conversation[];
      thread: ConversationThread | null;
      configuredModels: ConfiguredModel[];
    }
  | { type: "modelsLoaded"; configuredModels: ConfiguredModel[] }
  | { type: "modelSelected"; modelId: string | null }
  | { type: "selectionStarted"; conversationId: string }
  | { type: "selectionLoaded"; thread: ConversationThread }
  | { type: "newConversation" }
  | { type: "sendStarted" }
  | { type: "chatEvent"; event: ChatEvent }
  | { type: "sendFinished" }
  | { type: "failed"; message: string };

const initialState: ChatControllerState = {
  conversations: [],
  configuredModels: [],
  selectedModelId: null,
  activeConversationId: null,
  messages: [],
  isLoading: true,
  isSending: false,
  error: null,
};

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
    event.kind === "assistantStarted" ||
    event.kind === "assistantDelta" ||
    event.kind === "assistantCompleted"
  ) {
    requestConversationScrollToEnd(
      event.kind === "assistantDelta" ? event.conversationId : event.message.conversationId,
    );
  }
}

function reducer(state: ChatControllerState, action: ChatAction): ChatControllerState {
  switch (action.type) {
    case "initialised":
      return {
        ...state,
        conversations: action.conversations,
        configuredModels: action.configuredModels,
        selectedModelId:
          action.thread?.conversation.selectedModelId ?? action.configuredModels[0]?.id ?? null,
        activeConversationId: action.thread?.conversation.id ?? null,
        messages: action.thread?.messages ?? [],
        isLoading: false,
        error: null,
      };
    case "modelsLoaded": {
      const selectedStillExists = action.configuredModels.some(
        (model) => model.id === state.selectedModelId,
      );
      return {
        ...state,
        configuredModels: action.configuredModels,
        selectedModelId: selectedStillExists
          ? state.selectedModelId
          : (action.configuredModels[0]?.id ?? null),
      };
    }
    case "modelSelected":
      return { ...state, selectedModelId: action.modelId };
    case "selectionStarted":
      return {
        ...state,
        activeConversationId: action.conversationId,
        messages: [],
        isLoading: true,
        error: null,
      };
    case "selectionLoaded":
      return {
        ...state,
        conversations: reconcileConversation(state.conversations, action.thread.conversation),
        activeConversationId: action.thread.conversation.id,
        selectedModelId: action.thread.conversation.selectedModelId,
        messages: action.thread.messages,
        isLoading: false,
        error: null,
      };
    case "newConversation":
      return {
        ...state,
        activeConversationId: null,
        messages: [],
        isLoading: false,
        error: null,
      };
    case "sendStarted":
      return { ...state, isSending: true, error: null };
    case "chatEvent": {
      const event = action.event;
      if (event.kind === "accepted") {
        const belongsToActiveThread =
          state.activeConversationId === null ||
          state.activeConversationId === event.conversation.id;
        return {
          ...state,
          conversations: reconcileConversation(state.conversations, event.conversation),
          activeConversationId: belongsToActiveThread
            ? event.conversation.id
            : state.activeConversationId,
          messages: belongsToActiveThread
            ? reconcileMessage(state.messages, event.message)
            : state.messages,
          error: belongsToActiveThread ? null : state.error,
        };
      }
      if (event.kind === "assistantStarted" || event.kind === "assistantCompleted") {
        return event.message.conversationId === state.activeConversationId
          ? { ...state, messages: reconcileMessage(state.messages, event.message) }
          : state;
      }
      if (event.kind === "assistantDelta") {
        if (event.conversationId !== state.activeConversationId) return state;
        return {
          ...state,
          messages: state.messages.map((message) =>
            message.id === event.messageId
              ? {
                  ...message,
                  content: message.content + event.delta,
                  status: "streaming",
                  updatedAt: Date.now(),
                }
              : message,
          ),
        };
      }
      if (event.conversationId !== state.activeConversationId) return state;
      return {
        ...state,
        error: event.message,
        messages: state.messages.map((message) =>
          message.id === event.messageId
            ? { ...message, status: "failed", errorMessage: event.message }
            : message,
        ),
      };
    }
    case "sendFinished":
      return { ...state, isSending: false };
    case "failed":
      return { ...state, isLoading: false, isSending: false, error: action.message };
  }
}

export function useChatController() {
  const [state, dispatch] = useReducer(reducer, initialState);
  const selectionVersion = useRef(0);

  useEffect(() => {
    let cancelled = false;

    const initialise = async () => {
      try {
        const [conversations, configuredModels] = await Promise.all([
          listConversations(),
          listConfiguredModels(),
        ]);
        const thread = conversations[0]
          ? await getConversationThread(conversations[0].id)
          : null;
        if (!cancelled) {
          dispatch({ type: "initialised", conversations, thread, configuredModels });
        }
      } catch (error) {
        if (!cancelled) dispatch({ type: "failed", message: errorMessage(error) });
      }
    };

    void initialise();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void onModelsChanged(() => {
      void listConfiguredModels().then((configuredModels) => {
        if (!cancelled) dispatch({ type: "modelsLoaded", configuredModels });
      });
    }).then((stopListening) => {
      if (cancelled) stopListening();
      else unlisten = stopListening;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const selectConversation = useCallback(async (conversationId: string) => {
    const version = ++selectionVersion.current;
    dispatch({ type: "selectionStarted", conversationId });
    try {
      const thread = await getConversationThread(conversationId);
      if (version === selectionVersion.current) dispatch({ type: "selectionLoaded", thread });
    } catch (error) {
      if (version === selectionVersion.current) {
        dispatch({ type: "failed", message: errorMessage(error) });
      }
    }
  }, []);

  const startNewConversation = useCallback(() => {
    selectionVersion.current += 1;
    dispatch({ type: "newConversation" });
  }, []);

  const selectModel = useCallback((modelId: string | null) => {
    dispatch({ type: "modelSelected", modelId });
  }, []);

  const send = useCallback(
    async (content: string) => {
      if (state.isSending || content.trim().length === 0) return;
      dispatch({ type: "sendStarted" });
      try {
        const handleChatEvent = (event: ChatEvent) => {
          dispatch({ type: "chatEvent", event });
          requestScrollForChatEvent(event);
        };
        const accepted = await submitMessage(
          {
            conversationId: state.activeConversationId,
            configuredModelId: state.selectedModelId,
            content,
          },
          handleChatEvent,
        );
        handleChatEvent(acceptedEvent(accepted));
      } catch (error) {
        dispatch({ type: "failed", message: errorMessage(error) });
      } finally {
        dispatch({ type: "sendFinished" });
      }
    },
    [state.activeConversationId, state.isSending, state.selectedModelId],
  );

  const activeConversation = useMemo(
    () =>
      state.conversations.find((conversation) => conversation.id === state.activeConversationId) ??
      null,
    [state.activeConversationId, state.conversations],
  );

  return {
    ...state,
    activeConversation,
    selectConversation,
    startNewConversation,
    selectModel,
    send,
  };
}
