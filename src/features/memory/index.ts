// The memory module — Muninn inside Companion. The organ lives on :8002
// (/api/v1/memory); this module is everything Companion does with it: the
// organ client, the account-token seam, the toggleable reflex system the send
// path runs, and the /sleep pass.

export {
  clearAccountToken,
  loadAccountToken,
  saveAccountToken,
  type MemoryAgent,
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
  isMemoryEnabled,
  memoryPrefs,
  PREF_MEMORY_ENABLED,
  reflexPrefKey,
  reflexSetting,
  setMemoryPref,
  type MemoryPrefs,
} from "./prefs";
export { memoryReflexes } from "./reflexes/registry";
export type { MemoryReflex, ReflexRunReport } from "./reflexes/types";
export { sleepConversation, type SleepOutcome } from "./sleepService";
