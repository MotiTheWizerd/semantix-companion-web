import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";

import { getConversationThread, listConversations, submitMessage } from "./chatService";
import type {
  AcceptedMessage,
  ChatEvent,
  ChatMessage,
  Conversation,
  ConversationThread,
} from "./types";

interface ChatControllerState {
  conversations: Conversation[];
  activeConversationId: string | null;
  messages: ChatMessage[];
  isLoading: boolean;
  isSending: boolean;
  error: string | null;
}

type ChatAction =
  | { type: "initialised"; conversations: Conversation[]; thread: ConversationThread | null }
  | { type: "selectionStarted"; conversationId: string }
  | { type: "selectionLoaded"; thread: ConversationThread }
  | { type: "newConversation" }
  | { type: "sendStarted" }
  | { type: "chatEvent"; event: ChatEvent }
  | { type: "sendFinished" }
  | { type: "failed"; message: string };

const initialState: ChatControllerState = {
  conversations: [],
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

function reducer(state: ChatControllerState, action: ChatAction): ChatControllerState {
  switch (action.type) {
    case "initialised":
      return {
        ...state,
        conversations: action.conversations,
        activeConversationId: action.thread?.conversation.id ?? null,
        messages: action.thread?.messages ?? [],
        isLoading: false,
        error: null,
      };
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
        const conversations = await listConversations();
        const thread = conversations[0]
          ? await getConversationThread(conversations[0].id)
          : null;
        if (!cancelled) dispatch({ type: "initialised", conversations, thread });
      } catch (error) {
        if (!cancelled) dispatch({ type: "failed", message: errorMessage(error) });
      }
    };

    void initialise();
    return () => {
      cancelled = true;
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

  const send = useCallback(
    async (content: string) => {
      if (state.isSending || content.trim().length === 0) return;
      dispatch({ type: "sendStarted" });
      try {
        const accepted = await submitMessage(
          { conversationId: state.activeConversationId, content },
          (event) => dispatch({ type: "chatEvent", event }),
        );
        dispatch({ type: "chatEvent", event: acceptedEvent(accepted) });
      } catch (error) {
        dispatch({ type: "failed", message: errorMessage(error) });
      } finally {
        dispatch({ type: "sendFinished" });
      }
    },
    [state.activeConversationId, state.isSending],
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
    send,
  };
}
