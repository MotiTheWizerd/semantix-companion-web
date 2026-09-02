// The /sleep pass — runs in the Tauri backend, where the conversation lives
// and the model key never leaves the vault. The frontend resolves the agent
// identity (same brain recall reads from), watches the stage stream, and
// renders the outcome. Incremental since s487: the backend's ledger
// (messages.slept_at) means only NEW turns are distilled; nothingNew = the
// ledger already claims everything.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { companionMemoryAgent } from "./baseAgent";
import { loadAccountToken } from "./organService";
import { isAutoSleepEnabled, memoryPrefs } from "./prefs";

// ── the sleeper ───────────────────────────────────────────────────────────
// The pass that runs itself (s539): the backend watches each turn land and
// sleeps the thread once it is ripe — a dozen fresh turns, or a few and three
// minutes of quiet. The frontend's part is consent: name the brain on each
// send, or send nothing and leave the turn for a manual /sleep.

/** App-wide: the sleeper finished a pass nobody asked for. */
const SLEPT_EVENT = "memory://slept";

export type MemorySleptEvent =
  | {
      kind: "carved";
      conversationId: string;
      created: number;
      updated: number;
      dropped: number;
      memories: string[];
      scribeNote?: string;
    }
  | { kind: "failed"; conversationId: string; message: string };

/** The brain the sleeper may distil this send's thread into — null when the
 *  sleeper is off, memory is off, no account is connected, or the agent could
 *  not be resolved. Cached per companion underneath, so this is one keychain
 *  read on the send path. */
export async function autoSleepAgent(companionId: string | null): Promise<string | null> {
  if (!isAutoSleepEnabled(memoryPrefs([]))) return null;
  const token = await loadAccountToken().catch(() => null);
  if (!token) return null;
  try {
    const { agent_id: agentId } = await companionMemoryAgent(companionId);
    return agentId;
  } catch {
    return null;
  }
}

export function onMemorySlept(handler: (event: MemorySleptEvent) => void): Promise<UnlistenFn> {
  return listen<MemorySleptEvent>(SLEPT_EVENT, ({ payload }) => handler(payload));
}

// ── /sleep ────────────────────────────────────────────────────────────────

export interface SleepOutcome {
  created: number;
  updated: number;
  dropped: number;
  memories: string[];
  nothingNew: boolean;
  /** Present only when a model other than the companion's own distilled these
   *  memories — a Claude Code companion borrows the default model to write,
   *  since the organ's distiller needs an API key its login cannot provide. */
  scribeNote?: string;
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
  companionId: string | null,
  onProgress?: (event: SleepProgressEvent) => void,
): Promise<SleepOutcome> {
  // Sleep carves into the SAME companion recall reads from — the pairing that
  // [[decision-base-memory-one-brain-both-ways]] exists to protect.
  const agent = await companionMemoryAgent(companionId);
  const channel = new Channel<SleepProgressEvent>();
  channel.onmessage = (event) => onProgress?.(event);
  return invoke<SleepOutcome>("sleep_conversation", {
    conversationId,
    agentId: agent.agent_id,
    onProgress: channel,
  });
}
