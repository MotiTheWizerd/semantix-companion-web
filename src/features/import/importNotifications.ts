// The import's pulse — one live notification card per running job, patched in
// place as `import://progress` frames arrive (the same contract the /sleep
// card follows: post once as "active", keep patching the same id, land it).
//
// Mounted once at the app shell, so a run started from Settings keeps its
// card while the user wanders back to the conversation.

import type { UnlistenFn } from "@tauri-apps/api/event";

import { useNotificationsStore } from "../notifications/notificationsStore";
import {
  jobFinished,
  jobTotal,
  onImportProgress,
  type ImportProgress,
} from "./importService";

function cardId(jobId: string): string {
  return `import-${jobId}`;
}

function landedText(progress: ImportProgress): string {
  const memories =
    progress.memoriesCreated + progress.memoriesUpdated > 0
      ? `${progress.done} conversations became ${progress.memoriesCreated} new and ${progress.memoriesUpdated} updated memories.`
      : `${progress.done} conversations held nothing new to remember — that is normal for an archive.`;
  const failed =
    progress.failed > 0
      ? ` ${progress.failed} failed — retry them from Settings → Companions.`
      : "";
  return memories + failed;
}

export async function bindImportNotifications(): Promise<UnlistenFn> {
  return onImportProgress((progress) => {
    const { notify, updateNotification, notifications } =
      useNotificationsStore.getState();
    const id = cardId(progress.id);
    const total = jobTotal(progress);
    const finished = jobFinished(progress);

    if (progress.status === "running") {
      const patch = {
        title: "Importing history",
        text: progress.currentTitle
          ? `Remembering “${progress.currentTitle}”`
          : `${finished} of ${total} conversations`,
        status: "active" as const,
        progress: { done: finished, total },
      };
      if (notifications.some((notification) => notification.id === id)) {
        updateNotification(id, patch);
      } else {
        notify({ id, ...patch });
      }
      return;
    }

    // A terminal or parked frame lands the card. A job that never showed a
    // card (finished while the frontend was elsewhere) still gets its landing.
    const landing =
      progress.status === "done"
        ? { status: "success" as const, text: landedText(progress) }
        : progress.status === "paused"
          ? {
              status: "info" as const,
              text: "Import paused — resume any time from Settings → Companions.",
            }
          : progress.status === "cancelled"
            ? { status: "info" as const, text: "Import cancelled." }
            : {
                status: "error" as const,
                text:
                  progress.error ??
                  "The import stopped — resume it from Settings → Companions.",
              };
    if (notifications.some((notification) => notification.id === id)) {
      updateNotification(id, { progress: null, ...landing });
    } else {
      notify({ id, title: "Importing history", progress: null, ...landing });
    }
  });
}
