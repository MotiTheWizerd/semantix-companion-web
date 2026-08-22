// The /sleep pass — runs in the Tauri backend, where the conversation lives
// and the model key never leaves the vault. The frontend resolves the agent
// identity (same brain recall reads from), watches the stage stream, and
// renders the outcome. Incremental since s487: the backend's ledger
// (messages.slept_at) means only NEW turns are distilled; nothingNew = the
// ledger already claims everything.

import { Channel, invoke } from "@tauri-apps/api/core";

import { companionMemoryAgent } from "./baseAgent";

export interface SleepOutcome {
  created: number;
  updated: number;
  dropped: number;
  memories: string[];
  nothingNew: boolean;
}

/** Stage frames relayed verbatim from the organ's /sleep/stream. The terminal
 *  complete/error frames also pass through, but the invoke result is the
 *  authoritative outcome — callers only need the progress variants. */
export type SleepProgressEvent =
  | { type: "stage"; stage: "distilling"; turns: number }
  | { type: "stage"; stage: "carving"; total: number }
  | { type: "carved"; done: number; total: number; name: string }
  | { type: "complete" }
  | { type: "error"; detail?: string };

export async function sleepConversation(
  conversationId: string,
  onProgress?: (event: SleepProgressEvent) => void,
): Promise<SleepOutcome> {
  const agent = await companionMemoryAgent();
  const channel = new Channel<SleepProgressEvent>();
  channel.onmessage = (event) => onProgress?.(event);
  return invoke<SleepOutcome>("sleep_conversation", {
    conversationId,
    agentId: agent.agent_id,
    onProgress: channel,
  });
}
