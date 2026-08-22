// The /sleep pass — runs in the Tauri backend, where the conversation lives
// and the model key never leaves the vault. The frontend only resolves the
// agent identity (same brain recall reads from) and renders the outcome.

import { invoke } from "@tauri-apps/api/core";

import { companionMemoryAgent } from "./baseAgent";

export interface SleepOutcome {
  created: number;
  updated: number;
  dropped: number;
  memories: string[];
}

export async function sleepConversation(conversationId: string): Promise<SleepOutcome> {
  const agent = await companionMemoryAgent();
  return invoke<SleepOutcome>("sleep_conversation", {
    conversationId,
    agentId: agent.agent_id,
  });
}
