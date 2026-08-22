import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";

import {
  useNotificationsStore,
  type AppNotification,
} from "../features/notifications/notificationsStore";

// The notification stack — top-right pulse line. Renders the notifications
// store; posting/patching belongs to the features (first consumer: /sleep).

const AUTO_DISMISS_MS = 6000;

function Elapsed({ since }: { since: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.floor((Date.now() - since) / 1000));
  const label =
    seconds >= 60
      ? `${Math.floor(seconds / 60)}m ${seconds % 60}s`
      : `${seconds}s`;
  return <span className="notification-card__elapsed">{label}</span>;
}

function NotificationCard({ notification }: { notification: AppNotification }) {
  const dismiss = useNotificationsStore((s) => s.dismissNotification);

  // Landed cards fade themselves out; errors wait for the user's hand.
  useEffect(() => {
    if (notification.status !== "success" && notification.status !== "info") return;
    const timer = setTimeout(() => dismiss(notification.id), AUTO_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [notification.id, notification.status, dismiss]);

  const { progress } = notification;
  const percent =
    progress && progress.total > 0
      ? Math.round((progress.done / progress.total) * 100)
      : null;

  return (
    <div
      className={`notification-card notification-card--${notification.status}`}
      role="status"
    >
      <div className="notification-card__header">
        <span className="notification-card__dot" aria-hidden="true" />
        <span className="notification-card__title">{notification.title}</span>
        {notification.status === "active" && (
          <Elapsed since={notification.startedAt} />
        )}
        <button
          type="button"
          className="notification-card__dismiss"
          aria-label="Dismiss notification"
          onClick={() => dismiss(notification.id)}
        >
          ✕
        </button>
      </div>
      <div className="notification-card__text">{notification.text}</div>
      {percent !== null && (
        <div className="notification-card__track">
          <div
            className="notification-card__bar"
            style={{ width: `${percent}%` }}
          />
        </div>
      )}
    </div>
  );
}

export function NotificationStack() {
  const notifications = useNotificationsStore(
    useShallow((s) => s.notifications),
  );
  if (notifications.length === 0) return null;
  return (
    <div className="notification-stack">
      {notifications.map((notification) => (
        <NotificationCard key={notification.id} notification={notification} />
      ))}
    </div>
  );
}
