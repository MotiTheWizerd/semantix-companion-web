// Companion's memory agent — ONE private memory per companion, shared by the
// read side (recall reflexes) and the write side (/sleep). Both resolve the
// agent by the name stored on the companion record, so what sleep carves is
// what recall finds. Cached per agent name; a failed resolve is evicted so the
// next call retries instead of caching the outage.
//
// Which companion's memory is in play is the CALLER's answer now: pass the
// conversation's companion id and you get that companion's private pile. No id
// (a brand-new tab, or a companion since deleted) resolves to the built-in one
// — the same fallback Rust applies, so the two halves can never disagree about
// whose memory a send just touched.

import { listCompanions } from "../companions/companionService";
import type { Companion } from "../companions/types";
import { ensureMemoryAgent, type MemoryAgent } from "./organService";

/** The agent the app used before companions existed. It is also the built-in
 *  companion's agent, which is what makes it a safe fallback: an unreadable
 *  roster still lands on the same pile rather than a fresh, empty one. */
const FALLBACK_AGENT_NAME = "companion";
const AGENT_DESCRIPTION = "Companion memory — grown by /sleep";

const agentsByName = new Map<string, Promise<MemoryAgent>>();

/** Whose memory is in play: the named companion, else the built-in one. */
async function activeCompanion(companionId: string | null): Promise<Companion | null> {
  try {
    const roster = await listCompanions();
    return (
      (companionId ? roster.find((companion) => companion.id === companionId) : undefined) ??
      roster.find((companion) => companion.isBuiltIn) ??
      roster[0] ??
      null
    );
  } catch {
    // Fail open: memory keeps riding the historical agent rather than dying.
    return null;
  }
}

export async function companionMemoryAgent(
  companionId: string | null = null,
): Promise<MemoryAgent> {
  const companion = await activeCompanion(companionId);
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
