import { useCallback, useEffect, useRef, useState } from "react";

import { requestConversationScrollToEnd } from "../chat/chatScrollEvents";
import { listConversationCalls, onCallsChanged } from "./callService";
import type { CallThread } from "./types";

interface ConversationCallsState {
  threads: CallThread[];
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
  const [error, setError] = useState<string | null>(null);
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
    if (!conversationId) {
      setThreads([]);
      setError(null);
      return;
    }
    let active = true;
    void load(conversationId, () => active);
    return () => {
      active = false;
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

  return { threads, error };
}
