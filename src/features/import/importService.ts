// History import — the frontend's door to the Rust import rail.
//
// Everything heavy lives behind these invokes: parsing runs locally in Rust,
// distilling runs on the user's own model key, and the LEDGER in SQLite is
// what makes a thousands-of-conversations run pausable and restart-safe. The
// frontend only asks questions (inspect, list) and presses buttons (start,
// pause, resume, cancel, retry) — progress arrives as `import://progress`
// events, one fresh snapshot per conversation.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ImportSource = "claude" | "chatgpt";

export type ImportJobStatus =
  | "running"
  | "paused"
  | "done"
  | "cancelled"
  | "failed";

/** What the preview shows before anything is imported. */
export interface ImportInspection {
  source: ImportSource;
  conversations: number;
  totalTurns: number;
  emptySkipped: number;
  earliestMs: number | null;
  latestMs: number | null;
  /** Claude only: memories.json blobs riding along in the export. */
  claudeMemories: number;
}

/** One import job with its ledger counts — what the jobs list renders. */
export interface ImportJobSnapshot {
  id: string;
  companionId: string;
  agentRef: string;
  source: ImportSource;
  sourcePath: string;
  status: ImportJobStatus;
  error: string | null;
  includeClaudeMemories: boolean;
  createdAt: number;
  updatedAt: number;
  pending: number;
  done: number;
  failed: number;
  skipped: number;
  memoriesCreated: number;
  memoriesUpdated: number;
}

/** One `import://progress` frame: the snapshot plus what is on the table. */
export interface ImportProgress extends ImportJobSnapshot {
  currentTitle: string | null;
}

export interface StartImportOptions {
  includeClaudeMemories: boolean;
  /** Present = "only the last year"; absent = everything, which IS the product. */
  sinceMs?: number;
}

export function inspectImportSource(path: string): Promise<ImportInspection> {
  return invoke<ImportInspection>("inspect_import_source", { path });
}

export function startImport(
  path: string,
  companionId: string,
  agentId: string,
  options: StartImportOptions,
): Promise<ImportJobSnapshot> {
  return invoke<ImportJobSnapshot>("start_import", {
    path,
    companionId,
    agentId,
    options,
  });
}

export function pauseImport(jobId: string): Promise<void> {
  return invoke("pause_import", { jobId });
}

export function resumeImport(jobId: string): Promise<ImportJobSnapshot> {
  return invoke<ImportJobSnapshot>("resume_import", { jobId });
}

export function cancelImport(jobId: string): Promise<void> {
  return invoke("cancel_import", { jobId });
}

export function retryFailedImport(jobId: string): Promise<ImportJobSnapshot> {
  return invoke<ImportJobSnapshot>("retry_failed_import", { jobId });
}

export function listImportJobs(): Promise<ImportJobSnapshot[]> {
  return invoke<ImportJobSnapshot[]>("list_import_jobs");
}

export function onImportProgress(
  handler: (progress: ImportProgress) => void,
): Promise<UnlistenFn> {
  return listen<ImportProgress>("import://progress", (event) =>
    handler(event.payload),
  );
}

/** Conversations the job set out to handle, however each one lands. */
export function jobTotal(job: ImportJobSnapshot): number {
  return job.pending + job.done + job.failed + job.skipped;
}

/** Conversations already off the queue — the progress bar's numerator. */
export function jobFinished(job: ImportJobSnapshot): number {
  return job.done + job.failed + job.skipped;
}

/** "Jul 2023" — the preview's era endpoints. */
export function monthYear(ms: number | null): string | null {
  if (!ms || ms <= 0) return null;
  return new Date(ms).toLocaleDateString("en-US", {
    month: "short",
    year: "numeric",
  });
}
