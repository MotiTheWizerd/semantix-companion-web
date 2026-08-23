// Companion's memory agent — ONE private memory per companion, shared by the
// read side (recall reflexes) and the write side (/sleep). Both resolve the
// agent by the name stored on the companion record, so what sleep carves is
// what recall finds. Cached per agent name; a failed resolve is evicted so the
// next call retries instead of caching the outage.
//
// Until a conversation can name its own companion, every call resolves the
// BUILT-IN one. That single choice lives in activeCompanion() — wiring a
// conversation to its companion is a change there and nowhere else.

import { listCompanions } from "../companions/companionService";
import type { Companion } from "../companions/types";
import { ensureMemoryAgent, type MemoryAgent } from "./organService";

/** The agent the app used before companions existed. It is also the built-in
 *  companion's agent, which is what makes it a safe fallback: an unreadable
 *  roster still lands on the same pile rather than a fresh, empty one. */
const FALLBACK_AGENT_NAME = "companion";
const AGENT_DESCRIPTION = "Companion memory — grown by /sleep";

const agentsByName = new Map<string, Promise<MemoryAgent>>();

/** Whose memory is in play. One companion today; the conversation's pick next. */
async function activeCompanion(): Promise<Companion | null> {
  try {
    const roster = await listCompanions();
    return roster.find((companion) => companion.isBuiltIn) ?? roster[0] ?? null;
  } catch {
    // Fail open: memory keeps riding the historical agent rather than dying.
    return null;
  }
}

export async function companionMemoryAgent(): Promise<MemoryAgent> {
  const companion = await activeCompanion();
  const agentName = companion?.memoryAgentName ?? FALLBACK_AGENT_NAME;

  const cached = agentsByName.get(agentName);
  if (cached) return cached;

  const pending = ensureMemoryAgent(agentName, AGENT_DESCRIPTION).catch((error: unknown) => {
    agentsByName.delete(agentName);
    throw error;
  });
  agentsByName.set(agentName, pending);
  return pending;
}
