// Companion's memory agent — ONE brain, shared by the read side (recall
// reflexes) and the write side (/sleep). Both resolve the agent by name, so
// what sleep carves is what recall finds. Cached; a failed resolve is evicted
// so the next call retries instead of caching the outage.

import { ensureMemoryAgent, type MemoryAgent } from "./organService";

const AGENT_NAME = "companion";
const AGENT_DESCRIPTION = "Companion memory — grown by /sleep";

let pending: Promise<MemoryAgent> | null = null;

export function companionMemoryAgent(): Promise<MemoryAgent> {
  if (!pending) {
    pending = ensureMemoryAgent(AGENT_NAME, AGENT_DESCRIPTION).catch((error: unknown) => {
      pending = null;
      throw error;
    });
  }
  return pending;
}
