import { CompanionMark } from "./CompanionMark";

function PlusIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="M10 4.5v11M4.5 10h11" />
    </svg>
  );
}

function ChatIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="M4.25 5.25A2.25 2.25 0 0 1 6.5 3h7A2.25 2.25 0 0 1 15.75 5.25v5.5A2.25 2.25 0 0 1 13.5 13H9l-3.6 3v-3.1a2.25 2.25 0 0 1-1.15-1.97V5.25Z" />
    </svg>
  );
}

function MemoryIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="M10 3.25c-3.45 0-6.25 1.64-6.25 3.67 0 2.02 2.8 3.66 6.25 3.66s6.25-1.64 6.25-3.66c0-2.03-2.8-3.67-6.25-3.67Z" />
      <path d="M3.75 6.92v3.5c0 2.02 2.8 3.66 6.25 3.66s6.25-1.64 6.25-3.66v-3.5M3.75 10.42v2.66c0 2.03 2.8 3.67 6.25 3.67 1.45 0 2.78-.29 3.84-.78" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <circle cx="10" cy="10" r="3" />
      <circle cx="10" cy="10" r="6.25" />
      <path d="M10 1.75v2M10 16.25v2M1.75 10h2M16.25 10h2M4.17 4.17l1.42 1.42M14.41 14.41l1.42 1.42M15.83 4.17l-1.42 1.42M5.59 14.41l-1.42 1.42" />
    </svg>
  );
}

interface AppSidebarProps {
  activeView: "chat" | "settings";
  onViewChange: (view: "chat" | "settings") => void;
}

export function AppSidebar({ activeView, onViewChange }: AppSidebarProps) {
  return (
    <aside className="app-sidebar" aria-label="Companion navigation">
      <div className="sidebar-brand">
        <CompanionMark />
        <span>Companion</span>
      </div>

      <button className="new-chat-button" type="button">
        <PlusIcon />
        <span>New conversation</span>
      </button>

      <nav className="sidebar-navigation" aria-label="Primary navigation">
        <button
          className={`sidebar-navigation__item ${activeView === "chat" ? "is-active" : ""}`}
          type="button"
          aria-current={activeView === "chat" ? "page" : undefined}
          onClick={() => onViewChange("chat")}
        >
          <ChatIcon />
          <span>Chat</span>
        </button>
        <a className="sidebar-navigation__item" href="#memory">
          <MemoryIcon />
          <span>Memory</span>
          <span className="sidebar-navigation__hint">Soon</span>
        </a>
      </nav>

      <div className="conversation-list" aria-label="Recent conversations">
        <p className="sidebar-section-label">Today</p>
        <a className="conversation-list__item" href="#current-conversation">
          A quiet beginning
        </a>
      </div>

      <div className="sidebar-footer">
        <button
          className={`sidebar-settings-button ${activeView === "settings" ? "is-active" : ""}`}
          type="button"
          aria-current={activeView === "settings" ? "page" : undefined}
          onClick={() => onViewChange("settings")}
        >
          <SettingsIcon />
          <span>Settings</span>
        </button>

        <div className="sidebar-profile">
          <span className="sidebar-profile__avatar">M</span>
          <span className="sidebar-profile__identity">
            <strong>Moti</strong>
            <small>Private workspace</small>
          </span>
          <span className="sidebar-profile__status" title="Companion ready" />
        </div>
      </div>
    </aside>
  );
}
