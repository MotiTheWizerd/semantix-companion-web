// The ONE call the send path makes into the memory system. Gates (account
// token → master switch → per-reflex prefs) are checked here and in the
// registry. Fail-open end to end: any failure still lets the send proceed
// memoryless — but a failure AFTER the gates is reported on the chip, never
// swallowed. Silence is what hid the s483 CORS break; the chip is the
// instrument that keeps memory visible.

import type { ChatMessage } from "../chat/types";
import { companionMemoryAgent } from "./baseAgent";
import { loadAccountToken } from "./organService";
import { isMemoryEnabled, memoryPrefs } from "./prefs";
import { memoryReflexes, runPreSendReflexes } from "./reflexes/registry";

export interface MemoryPreSendArgs {
  conversationId: string | null;
  /** The user's typed message. */
  text: string;
  /** The PRIOR conversation messages (loaded runtime), oldest first. */
  messages: ChatMessage[];
}

/** What the 🧠 chip renders for one sent message — instrument data only,
 *  runtime-held, never persisted (the bodies rode the inference request). */
export interface MemoryRecallChipData {
  hits: { name: string; memType: string; score: number; mirror: boolean }[];
  /** false = the organ answered on the keyword leg alone; null = no recall. */
  vectorLeg: boolean | null;
  /** Set when the pass died before the reflexes ran (organ unreachable). */
  failure: string | null;
  errors: { reflexId: string; message: string }[];
}

export interface MemoryRide {
  /** Everything that rides ahead of the user's message; '' = nothing. */
  injection: string;
  chip: MemoryRecallChipData;
}

/** null = memory did not ride AND has nothing to show (no account, gated off).
 *  A ride with a `failure` chip = memory was on but the organ was out of
 *  reach; the send proceeds memoryless and the chip says so. */
export async function runMemoryPreSend(
  args: MemoryPreSendArgs,
): Promise<MemoryRide | null> {
  const token = await loadAccountToken().catch(() => null);
  if (!token) return null;

  const prefs = memoryPrefs(memoryReflexes.map((reflex) => reflex.id));
  if (!isMemoryEnabled(prefs)) return null;

  try {
    const { agent_id: agentId } = await companionMemoryAgent();

    let lastAssistantText: string | null = null;
    let lastUserAtMs: number | null = null;
    for (const message of args.messages) {
      if (message.status !== "completed" || !message.content.trim()) continue;
      if (message.role === "assistant") lastAssistantText = message.content;
      else if (message.role === "user") lastUserAtMs = message.createdAt;
    }

    const report = await runPreSendReflexes(
      {
        conversationId: args.conversationId,
        agentId,
        text: args.text,
        lastAssistantText,
        nowMs: Date.now(),
        lastUserAtMs,
      },
      prefs,
    );
    if (report.ran.length === 0) return null;
    const toChipHit = (
      hit: { memory: { name: string; mem_type: string }; score: number },
      mirror: boolean,
    ) => ({ name: hit.memory.name, memType: hit.memory.mem_type, score: hit.score, mirror });
    return {
      injection: report.injection,
      chip: {
        hits: [
          ...report.hits.map((hit) => toChipHit(hit, false)),
          ...report.mirrorHits.map((hit) => toChipHit(hit, true)),
        ],
        vectorLeg: report.vectorLeg,
        failure: null,
        errors: report.errors,
      },
    };
  } catch (error) {
    return {
      injection: "",
      chip: {
        hits: [],
        vectorLeg: null,
        failure: error instanceof Error ? error.message : String(error),
        errors: [],
      },
    };
  }
}
