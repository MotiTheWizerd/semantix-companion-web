// Memory preference layer — Companion keeps these client-side in localStorage
// (the studio rides its preference store; the keys and semantics match).
// Two gates stack: memory.enabled (master switch, default ON once an account
// token exists) and memory.reflex.<id> per reflex. Token presence itself is
// the outer consent gate — checked in preSend, not here, because it's async.

const STORAGE_PREFIX = "companion.";

export const PREF_MEMORY_ENABLED = "memory.enabled";
/** The sleeper: distil a conversation on its own once it is ripe, instead of
 *  waiting for /sleep. Under the master switch; default ON. */
export const PREF_AUTO_SLEEP = "memory.autoSleep";

export type MemoryPrefs = Record<string, unknown>;

function read(key: string): unknown {
  try {
    const raw = localStorage.getItem(STORAGE_PREFIX + key);
    return raw === null ? undefined : (JSON.parse(raw) as unknown);
  } catch {
    return undefined;
  }
}

/** Snapshot of every memory pref the reflex registry reads. */
export function memoryPrefs(reflexIds: string[]): MemoryPrefs {
  const prefs: MemoryPrefs = {
    [PREF_MEMORY_ENABLED]: read(PREF_MEMORY_ENABLED),
    [PREF_AUTO_SLEEP]: read(PREF_AUTO_SLEEP),
  };
  for (const id of reflexIds) {
    const key = reflexPrefKey(id);
    prefs[key] = read(key);
  }
  return prefs;
}

export function setMemoryPref(key: string, value: boolean): void {
  try {
    localStorage.setItem(STORAGE_PREFIX + key, JSON.stringify(value));
  } catch {
    /* storage unavailable — the toggle just won't persist */
  }
}

/** The feature master switch — default ON (an empty roster recalls nothing,
 *  so the organ is inert until memories exist). */
export function isMemoryEnabled(prefs: MemoryPrefs): boolean {
  return prefs[PREF_MEMORY_ENABLED] !== false;
}

/** The sleeper is on unless turned off — and never while memory itself is off. */
export function isAutoSleepEnabled(prefs: MemoryPrefs): boolean {
  return isMemoryEnabled(prefs) && prefs[PREF_AUTO_SLEEP] !== false;
}

export interface ReflexSetting {
  enabled: boolean;
  // future: effort?: 'low' | 'medium' | 'high'
}

export function reflexPrefKey(reflexId: string): string {
  return `memory.reflex.${reflexId}`;
}

export function reflexSetting(
  prefs: MemoryPrefs,
  reflex: { id: string; defaultOn: boolean },
): ReflexSetting {
  const value = prefs[reflexPrefKey(reflex.id)];
  return { enabled: typeof value === "boolean" ? value : reflex.defaultOn };
}
