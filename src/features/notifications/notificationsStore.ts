import { create } from "zustand";

// Notifications — the Companion's pulse line (same contract as the studio's
// s487 slice). A notification is a LIVE OBJECT, not a fire-and-forget toast:
// a long-running job posts once with status "active" and keeps patching the
// same id as it moves through stages, then lands it as "success" or "error".
// Active cards are sticky; success/info auto-dismiss (the card owns the
// timer); errors stay until dismissed by hand.
// First consumer: the /sleep memory pass.

export type NotificationStatus = "active" | "success" | "error" | "info";

export interface AppNotification {
  id: string;
  /** Header line, e.g. "Sleep". */
  title: string;
  /** Current body line — for live jobs, the stage narration. */
  text: string;
  status: NotificationStatus;
  /** Determinate progress (carving 3/11); null = indeterminate. */
  progress: { done: number; total: number } | null;
  /** Epoch ms — the card's elapsed ticker runs off this while active. */
  startedAt: number;
}

interface NotificationsState {
  notifications: AppNotification[];
  /** Post a notification; returns its id so the caller can patch it later. */
  notify: (
    notification: Omit<AppNotification, "id" | "startedAt" | "progress"> & {
      id?: string;
      progress?: { done: number; total: number } | null;
    },
  ) => string;
  /** Patch a live notification in place (stage text, progress, final status). */
  updateNotification: (
    id: string,
    patch: Partial<Omit<AppNotification, "id">>,
  ) => void;
  dismissNotification: (id: string) => void;
}

export const useNotificationsStore = create<NotificationsState>((set) => ({
  notifications: [],

  notify: (notification) => {
    const id = notification.id ?? crypto.randomUUID();
    const complete: AppNotification = {
      progress: null,
      ...notification,
      id,
      startedAt: Date.now(),
    };
    set((state) => ({
      notifications: [
        ...state.notifications.filter((n) => n.id !== id),
        complete,
      ],
    }));
    return id;
  },

  updateNotification: (id, patch) => {
    set((state) => {
      const index = state.notifications.findIndex((n) => n.id === id);
      if (index === -1) return state;
      const notifications = state.notifications.slice();
      notifications[index] = { ...notifications[index], ...patch };
      return { notifications };
    });
  },

  dismissNotification: (id) => {
    set((state) => ({
      notifications: state.notifications.filter((n) => n.id !== id),
    }));
  },
}));
