import { useCallback, useEffect, useRef, useState } from "react";

import { requestConversationScrollToEnd } from "../chat/chatScrollEvents";
import { listConversationCalls, onCallsChanged, onCallSpeech, onCallWake } from "./callService";
import type { CallThread, StreamingCallMessage } from "./types";

const NOBODY_REPLYING: ReadonlyMap<string, string> = new Map();

interface ConversationCallsState {
  threads: CallThread[];
  streamingMessages: StreamingCallMessage[];
  /** callId → agentId for every call whose reply is in flight right now: the
   * waker started a turn for it and that turn has not ended. Armed by
   * `calls://wake`, disarmed by `calls://changed` — Rust guarantees the second
   * always follows the first, so this cannot be left stale. */
  replyingByCallId: ReadonlyMap<string, string>;
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
  const [replyingByCallId, setReplyingByCallId] =
    useState<ReadonlyMap<string, string>>(NOBODY_REPLYING);
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
    setReplyingByCallId(NOBODY_REPLYING);
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

  // The reply-in-flight lane. The waker announces the woken turn the moment
  // it starts; the card shows a typing ghost for that call until the turn
  // ends. Filtered by the loaded thread set, same as speech — call ids are
  // globally unique and other windows share this bus.
  useEffect(() => {
    if (!conversationId) return;
    let active = true;
    const subscription = onCallWake((event) => {
      if (!active) return;
      if (!knownCallIds.current.get(conversationId)?.has(event.callId)) return;
      setReplyingByCallId((current) =>
        new Map(current).set(event.callId, event.agentId),
      );
    });
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten());
    };
  }, [conversationId]);

  // The woken lane. A companion answered a call with nobody watching, and
  // this is how the card learns about it — the only path here that is not
  // downstream of something the user did. It is also the falling edge of
  // every wake: woken turns run one at a time and each ends with this event,
  // so clearing the whole in-flight set here is exact, not approximate.
  useEffect(() => {
    if (!conversationId) return;
    let active = true;
    const subscription = onCallsChanged(() => {
      if (!active) return;
      setReplyingByCallId(NOBODY_REPLYING);
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
    replyingByCallId,
    isInitialLoading:
      conversationId !== null && loadedConversationId !== conversationId,
    error,
  };
}
