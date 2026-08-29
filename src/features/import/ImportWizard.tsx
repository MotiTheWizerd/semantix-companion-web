// "Bring your history" — the wizard that walks a NON-TECHNICAL user from
// "my old chats live somewhere in Claude/ChatGPT" to a companion that
// remembers them. Three moments, in the order the user actually meets them:
//
//   1. GET the file — most people have never exported anything; teach it.
//   2. DROP the file — the zip as downloaded, the folder it unzipped into,
//      or a bare conversations.json. The source is sniffed, never asked.
//   3. PREVIEW & GO — "Found 845 conversations, Jul 2023 → Jul 2026",
//      import everything (the product) or only the last year, then the run
//      continues in the background with a live card; pause/resume/retry
//      live in the jobs list below.
//
// Honest copy throughout: parsed on this machine, distilled on YOUR model
// key, takes a while for a big archive.

import { useCallback, useEffect, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import type { UnlistenFn } from "@tauri-apps/api/event";

import { companionMemoryAgent } from "../memory/baseAgent";
import { companionLabel, type Companion } from "../companions/types";
import {
  cancelImport,
  inspectImportSource,
  jobFinished,
  jobTotal,
  listImportJobs,
  monthYear,
  onImportProgress,
  pauseImport,
  resumeImport,
  retryFailedImport,
  startImport,
  type ImportInspection,
  type ImportJobSnapshot,
} from "./importService";

interface ImportWizardProps {
  companion: Companion;
  onClose: () => void;
}

type Scope = "everything" | "lastYear";

const YEAR_MS = 365 * 24 * 60 * 60 * 1000;

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "The import ran into a problem.";
}

function sourceLabel(source: "claude" | "chatgpt"): string {
  return source === "claude" ? "Claude" : "ChatGPT";
}

function previewLine(inspection: ImportInspection): string {
  const count = `Found ${inspection.conversations.toLocaleString()} conversations`;
  const from = monthYear(inspection.earliestMs);
  const to = monthYear(inspection.latestMs);
  return from && to ? `${count}, ${from} → ${to}` : count;
}

function jobStatusLine(job: ImportJobSnapshot): string {
  const progress = `${jobFinished(job).toLocaleString()} of ${jobTotal(job).toLocaleString()}`;
  switch (job.status) {
    case "running":
      return `Importing — ${progress}`;
    case "paused":
      return `Paused at ${progress}`;
    case "done":
      return `Done — ${job.memoriesCreated + job.memoriesUpdated} memories from ${job.done} conversations`;
    case "cancelled":
      return `Cancelled at ${progress}`;
    case "failed":
      return job.error ?? "Stopped — resume to continue";
  }
}

export function ImportWizard({ companion, onClose }: ImportWizardProps) {
  const [path, setPath] = useState<string | null>(null);
  const [inspection, setInspection] = useState<ImportInspection | null>(null);
  const [scope, setScope] = useState<Scope>("everything");
  const [includeMemories, setIncludeMemories] = useState(true);
  const [isInspecting, setIsInspecting] = useState(false);
  const [isStarting, setIsStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [jobs, setJobs] = useState<ImportJobSnapshot[]>([]);
  const [busyJobId, setBusyJobId] = useState<string | null>(null);

  const refreshJobs = useCallback(async () => {
    try {
      const all = await listImportJobs();
      setJobs(all.filter((job) => job.companionId === companion.id));
    } catch {
      // The list is a convenience; a failed refresh never blocks the wizard.
    }
  }, [companion.id]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | undefined;
    void refreshJobs();
    void onImportProgress(() => {
      if (!cancelled) void refreshJobs();
    }).then((stop) => {
      if (cancelled) stop();
      else unlisten = stop;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refreshJobs]);

  const inspect = async (picked: string) => {
    setError(null);
    setIsInspecting(true);
    try {
      const result = await inspectImportSource(picked);
      setPath(picked);
      setInspection(result);
      setScope("everything");
      setIncludeMemories(true);
    } catch (inspectError) {
      setError(errorMessage(inspectError));
    } finally {
      setIsInspecting(false);
    }
  };

  const pickZip = async () => {
    const picked = await openFileDialog({
      title: "Choose your export (.zip or conversations.json)",
      filters: [{ name: "Chat export", extensions: ["zip", "json"] }],
    });
    if (typeof picked === "string" && picked) await inspect(picked);
  };

  const pickFolder = async () => {
    const picked = await openFileDialog({
      directory: true,
      title: "Choose the folder your export unzipped into",
    });
    if (typeof picked === "string" && picked) await inspect(picked);
  };

  const begin = async () => {
    if (!path || !inspection) return;
    setError(null);
    setIsStarting(true);
    try {
      const agent = await companionMemoryAgent(companion.id);
      await startImport(path, companion.id, agent.agent_id, {
        includeClaudeMemories: includeMemories && inspection.claudeMemories > 0,
        ...(scope === "lastYear" ? { sinceMs: Date.now() - YEAR_MS } : {}),
      });
      // The run belongs to the background now: the live card narrates it and
      // the jobs list below owns pause/resume. Clear the picker for a fresh
      // drop rather than leaving a stale preview around.
      setPath(null);
      setInspection(null);
      await refreshJobs();
    } catch (startError) {
      setError(errorMessage(startError));
    } finally {
      setIsStarting(false);
    }
  };

  const act = async (jobId: string, action: (id: string) => Promise<unknown>) => {
    setError(null);
    setBusyJobId(jobId);
    try {
      await action(jobId);
      await refreshJobs();
    } catch (actionError) {
      setError(errorMessage(actionError));
    } finally {
      setBusyJobId(null);
    }
  };

  return (
    <section className="import-wizard" aria-labelledby="import-wizard-title">
      <div className="credential-form__heading">
        <div>
          <h3 id="import-wizard-title">
            Bring your history to {companionLabel(companion)}
          </h3>
          <p>
            Import your old Claude or ChatGPT conversations, and this companion
            will remember what matters from them — in your own words, in your
            own language.
          </p>
        </div>
        <button
          className="credential-form__close"
          type="button"
          aria-label="Close the import wizard"
          onClick={onClose}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="m7 7 10 10M17 7 7 17" />
          </svg>
        </button>
      </div>

      {inspection === null ? (
        <>
          <div className="import-wizard__teach">
            <h4>First, get your export file</h4>
            <p>
              Your chats live with Claude or ChatGPT until you ask for them.
              Both send you a download — it usually takes a few minutes to
              arrive by email.
            </p>
            <details>
              <summary>How to export from Claude</summary>
              <ol>
                <li>
                  In Claude (the app or claude.ai), open{" "}
                  <strong>Settings → Privacy</strong>.
                </li>
                <li>
                  Click <strong>Export data</strong>. Claude emails you a
                  download link.
                </li>
                <li>
                  Download the file from that email — it is a <code>.zip</code>.
                  You do not need to open or unzip it.
                </li>
              </ol>
            </details>
            <details>
              <summary>How to export from ChatGPT</summary>
              <ol>
                <li>
                  In ChatGPT, open <strong>Settings → Data controls</strong>.
                </li>
                <li>
                  Click <strong>Export data</strong> and confirm. ChatGPT emails
                  you a download link (it expires after a day, so download soon).
                </li>
                <li>
                  Download the file from that email — it is a <code>.zip</code>.
                  You do not need to open or unzip it.
                </li>
              </ol>
            </details>
          </div>

          <div className="import-wizard__drop">
            <h4>Then, drop it here</h4>
            <p>
              The zip exactly as it downloaded is perfect. Already unzipped it?
              The folder works just as well.
            </p>
            <div className="import-wizard__pickers">
              <button
                className="credential-button credential-button--primary"
                type="button"
                disabled={isInspecting}
                onClick={() => void pickZip()}
              >
                {isInspecting ? "Reading…" : "Choose the downloaded file"}
              </button>
              <button
                className="credential-button credential-button--quiet"
                type="button"
                disabled={isInspecting}
                onClick={() => void pickFolder()}
              >
                Choose an unzipped folder
              </button>
            </div>
            <p className="import-wizard__fineprint">
              Everything is read on this computer. Nothing is uploaded anywhere
              except to the model you already use, one conversation at a time.
            </p>
          </div>
        </>
      ) : (
        <div className="import-wizard__preview">
          <h4>
            {sourceLabel(inspection.source)} export — {previewLine(inspection)}
          </h4>
          <p>
            {inspection.totalTurns.toLocaleString()} messages
            {inspection.emptySkipped > 0
              ? ` · ${inspection.emptySkipped} empty conversations will be skipped`
              : ""}
          </p>

          <div className="import-wizard__choices" role="radiogroup" aria-label="What to import">
            <label>
              <input
                type="radio"
                name="import-scope"
                checked={scope === "everything"}
                onChange={() => setScope("everything")}
              />
              <span>
                <strong>Import everything</strong> — the full story, oldest
                first. Recommended.
              </span>
            </label>
            <label>
              <input
                type="radio"
                name="import-scope"
                checked={scope === "lastYear"}
                onChange={() => setScope("lastYear")}
              />
              <span>
                <strong>Only the last year</strong> — quicker, just the recent
                chapters.
              </span>
            </label>
            {inspection.claudeMemories > 0 ? (
              <label>
                <input
                  type="checkbox"
                  checked={includeMemories}
                  onChange={(event) => setIncludeMemories(event.target.checked)}
                />
                <span>
                  Also import what Claude already remembers about you
                  (memories.json)
                </span>
              </label>
            ) : null}
          </div>

          <p className="import-wizard__fineprint">
            Your conversations become searchable word-for-word within moments
            of starting. Distilling them into memories takes a while — hours,
            for years of chats. It runs in the background on your own model
            key, you can keep using the app, and you can pause and resume any
            time. Importing the same export twice only processes what changed.
          </p>

          <div className="credential-form__actions">
            <button
              className="credential-button credential-button--quiet"
              type="button"
              disabled={isStarting}
              onClick={() => {
                setPath(null);
                setInspection(null);
                setError(null);
              }}
            >
              Back
            </button>
            <button
              className="credential-button credential-button--primary"
              type="button"
              disabled={isStarting}
              onClick={() => void begin()}
            >
              {isStarting ? "Starting…" : "Start importing"}
            </button>
          </div>
        </div>
      )}

      {error ? (
        <p className="credential-store__error" role="alert">
          {error}
        </p>
      ) : null}

      {jobs.length > 0 ? (
        <div className="import-wizard__jobs">
          <h4>Imports for this companion</h4>
          <ul>
            {jobs.map((job) => (
              <li key={job.id}>
                <div className="import-wizard__job-identity">
                  <strong>{sourceLabel(job.source)} export</strong>
                  <span>{jobStatusLine(job)}</span>
                </div>
                <div className="import-wizard__job-actions">
                  {job.status === "running" ? (
                    <>
                      <button
                        className="credential-button credential-button--quiet"
                        type="button"
                        disabled={busyJobId === job.id}
                        onClick={() => void act(job.id, pauseImport)}
                      >
                        Pause
                      </button>
                      <button
                        className="credential-button credential-button--quiet"
                        type="button"
                        disabled={busyJobId === job.id}
                        onClick={() => void act(job.id, cancelImport)}
                      >
                        Cancel
                      </button>
                    </>
                  ) : null}
                  {job.status === "paused" || job.status === "failed" ? (
                    <>
                      <button
                        className="credential-button credential-button--primary"
                        type="button"
                        disabled={busyJobId === job.id}
                        onClick={() => void act(job.id, resumeImport)}
                      >
                        Resume
                      </button>
                      <button
                        className="credential-button credential-button--quiet"
                        type="button"
                        disabled={busyJobId === job.id}
                        onClick={() => void act(job.id, cancelImport)}
                      >
                        Cancel
                      </button>
                    </>
                  ) : null}
                  {job.status === "done" && job.failed > 0 ? (
                    <button
                      className="credential-button credential-button--quiet"
                      type="button"
                      disabled={busyJobId === job.id}
                      onClick={() => void act(job.id, retryFailedImport)}
                    >
                      Retry {job.failed} failed
                    </button>
                  ) : null}
                </div>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </section>
  );
}
