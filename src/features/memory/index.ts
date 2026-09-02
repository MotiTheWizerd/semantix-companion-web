// The memory module — Muninn inside Companion. The organ lives on :8002
// (/api/v1/memory); this module is everything Companion does with it: the
// organ client, the account-token seam, the toggleable reflex system the send
// path runs, and the /sleep pass.

export {
  clearAccountToken,
  loadAccountToken,
  loadMemoryGraph,
  readMemory,
  recallMemories,
  saveAccountToken,
  type MemoryAgent,
  type MemoryGraph,
  type MemoryGraphEdge,
  type MemoryGraphNode,
  type MemoryGraphOptions,
  type MemoryGraphStats,
  type MemoryHit,
  type MemoryRecord,
  type RecallResult,
} from "./organService";
export { companionMemoryAgent } from "./baseAgent";
export {
  runMemoryPreSend,
  type MemoryPreSendArgs,
  type MemoryRecallChipData,
  type MemoryRide,
} from "./preSend";
export {
  isAutoSleepEnabled,
  isMemoryEnabled,
  memoryPrefs,
  PREF_AUTO_SLEEP,
  PREF_MEMORY_ENABLED,
  reflexPrefKey,
  reflexSetting,
  setMemoryPref,
  type MemoryPrefs,
} from "./prefs";
export { memoryReflexes } from "./reflexes/registry";
export type { MemoryReflex, ReflexRunReport } from "./reflexes/types";
export {
  autoSleepAgent,
  onMemorySlept,
  sleepConversation,
  type MemorySleptEvent,
  type SleepOutcome,
} from "./sleepService";
