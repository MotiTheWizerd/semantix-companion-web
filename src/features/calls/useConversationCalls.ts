import { useCallback, useEffect, useRef, useState } from "react";

import { requestConversationScrollToEnd } from "../chat/chatScrollEvents";
import { listConversationCalls, onCallsChanged, onCallSpeech } from "./callService";
import type { CallThread, StreamingCallMessage } from "./types";

interface ConversationCallsState {
  threads: CallThread[];
  streamingMessages: StreamingCallMessage[];
  /** True only while this conversation's first authoritative call read is
   * pending. Refreshes do not blank or re-block the already rendered thread. */
  isInitialLoading: boolean;
  error: string | null;
}

/**
 * The calls born out of one conversation.
 *
 * ⚑ REFETCHES ON THE FALLING EDGE OF `turnInProgress`. A call can only change
 * as the result of a tool the model ran, so the end of a turn is the only
 * moment worth looking — polling would ask a hundred times to learn nothing.
 * This is the honest shape until Rust emits on write; when it does, the event
 * replaces this trigger and nothing outside this file has to change.
 *
 * The hook owns that policy so its host does not: a caller passes what it
 * already knows (which conversation, whether a turn is running) and never
 * learns how calls are fetched.
 */
export function useConversationCalls(
  conversationId: string | null,
  turnInProgress: boolean,
): ConversationCallsState {
  const [threads, setThreads] = useState<CallThread[]>([]);
  const [streamingMessages, setStreamingMessages] = useState<StreamingCallMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loadedConversationId, setLoadedConversationId] = useState<string | null>(null);
  const wasBusy = useRef(turnInProgress);
  const knownCallIds = useRef(new Map<string, Set<string>>());

  const load = useCallback(
    async (id: string, alive: () => boolean) => {
      try {
        const found = await listConversationCalls(id);
        if (!alive()) return;
        const previousIds = knownCallIds.current.get(id);
        const nextIds = new Set(found.map((thread) => thread.call.id));
        const hasNewCall = previousIds
          ? found.some((thread) => !previousIds.has(thread.call.id))
          : false;
        knownCallIds.current.set(id, nextIds);
        setThreads(found);
        // A successful send is in SQLite before Rust emits its finished edge.
        // Replace the transient draft only when that exact authoritative row
        // is visible, which prevents both duplication and a blank-frame flicker.
        setStreamingMessages((current) =>
          current.filter(
            (draft) =>
              !found.some((thread) =>
                thread.messages.some(
                  (message) =>
                    message.callId === draft.callId &&
                    message.fromAgentId === draft.fromAgentId &&
                    message.body.trim() === draft.body.trim(),
                ),
              ),
          ),
        );
        setError(null);
        // A newly opened call is a new transcript row, so follow it just like
        // a new message. Later call turns update that row in place and must not
        // yank the viewport to the end of a conversation that continued below.
        if (hasNewCall) requestConversationScrollToEnd(id);
      } catch (cause) {
        if (!alive()) return;
        // A failed read is shown, never swallowed — a silently empty call
        // list is indistinguishable from "nothing was said on your behalf",
        // and those two must never look the same.
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    },
    [],
  );

  useEffect(() => {
    setThreads([]);
    setStreamingMessages([]);
    setError(null);
    if (!conversationId) {
      setLoadedConversationId(null);
      return;
    }
    let active = true;
    void load(conversationId, () => active).finally(() => {
      if (active) setLoadedConversationId(conversationId);
    });
    return () => {
      active = false;
    };
  }, [conversationId, load]);

  useEffect(() => {
    if (!conversationId) return;
    let active = true;
    const subscription = onCallSpeech((event) => {
      if (!active) return;
      const callIds = knownCallIds.current.get(conversationId);
      if (event.kind === "callSpeechDelta") {
        // An event for another open tab shares the same app bus. Call ids are
        // globally unique; the loaded thread set is the conversation boundary.
        if (!callIds?.has(event.callId)) return;
        setStreamingMessages((current) => {
          const existing = current.find((draft) => draft.streamId === event.streamId);
          if (!existing) {
            return [
              ...current,
              {
                streamId: event.streamId,
                callId: event.callId,
                fromAgentId: event.fromAgentId,
                body: event.delta,
              },
            ];
          }
          return current.map((draft) =>
            draft.streamId === event.streamId
              ? { ...draft, body: draft.body + event.delta }
              : draft,
          );
        });
        return;
      }

      if (!callIds?.has(event.callId)) return;
      if (!event.succeeded) {
        setStreamingMessages((current) =>
          current.filter(
            (draft) =>
              draft.callId !== event.callId || draft.fromAgentId !== event.fromAgentId,
          ),
        );
        return;
      }
      void load(conversationId, () => active);
    });
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten());
    };
  }, [conversationId, load]);

  useEffect(() => {
    const finished = wasBusy.current && !turnInProgress;
    wasBusy.current = turnInProgress;
    if (!finished || !conversationId) return;
    let active = true;
    void load(conversationId, () => active);
    return () => {
      active = false;
    };
  }, [turnInProgress, conversationId, load]);

  // The woken lane. A companion answered a call with nobody watching, and
  // this is how the card learns about it — the only path here that is not
  // downstream of something the user did.
  useEffect(() => {
    if (!conversationId) return;
    let active = true;
    const subscription = onCallsChanged(() => {
      void load(conversationId, () => active);
    });
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten());
    };
  }, [conversationId, load]);

  return {
    threads,
    streamingMessages,
    isInitialLoading:
      conversationId !== null && loadedConversationId !== conversationId,
    error,
  };
}
