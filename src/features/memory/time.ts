// Time awareness for the memory system — ported from the studio module (s480/
// s481) so both products anchor models in "now" the same way. Two jobs:
//   1. AGES — every recalled memory carries "carved <age> (<date>)".
//   2. THE GAP REFLEX — when the gap since the user's previous message is
//      abnormal, the injected time header says so explicitly.
// Fail-open everywhere: a missing/unparseable/future-skewed stamp yields "".

/** Gaps shorter than this are normal chat rhythm — not worth a line. */
const GAP_NOTE_MS = 2 * 60_000;
/** Past this the gap is an anomaly — the user left and came back. */
const GAP_LONG_MS = 30 * 60_000;

/** "just now" / "14m ago" / "3h ago" / "12d ago" — "" on bad or future stamps. */
export function formatAge(iso: string, nowMs: number): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t) || t > nowMs + 60_000) return "";
  const m = Math.max(0, Math.floor((nowMs - t) / 60_000));
  if (m < 1) return "just now";
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

/** "45s" / "14m" / "3h 12m" / "2d 5h" — duration, not age. */
export function formatGap(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h${m % 60 ? ` ${m % 60}m` : ""}`;
  const d = Math.floor(h / 24);
  return `${d}d${h % 24 ? ` ${h % 24}h` : ""}`;
}

const NOW_FMT = new Intl.DateTimeFormat("en-GB", {
  weekday: "long",
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
});

/** The <time-awareness> block that rides ahead of the memory block: the
 *  now-anchor plus the gap reflex. `lastUserAtMs` = stamp of the user's
 *  PREVIOUS message, null on the first. */
export function composeTimeBlock(
  nowMs: number,
  lastUserAtMs: number | null,
): string {
  const lines = [`Current time: ${NOW_FMT.format(new Date(nowMs))}.`];
  if (lastUserAtMs !== null) {
    const gap = nowMs - lastUserAtMs;
    if (gap >= GAP_LONG_MS) {
      lines.push(
        `It has been ${formatGap(gap)} since the user's previous message — ` +
          `a long gap; time has passed in their world.`,
      );
    } else if (gap >= GAP_NOTE_MS) {
      lines.push(`Time since the user's previous message: ${formatGap(gap)}.`);
    }
  }
  return `<time-awareness>\n${lines.join("\n")}\n</time-awareness>\n\n`;
}
