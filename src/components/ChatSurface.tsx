import { EmptyState } from "./EmptyState";

function AttachIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="m7.75 10.75 4.4-4.4a2.3 2.3 0 1 1 3.25 3.25l-5.85 5.85a3.45 3.45 0 0 1-4.88-4.88l6.1-6.1" />
    </svg>
  );
}

function SendIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="M10 15.5v-11M5.75 8.75 10 4.5l4.25 4.25" />
    </svg>
  );
}

export function ChatSurface() {
  return (
    <main className="chat-surface" id="chat">
      <EmptyState />

      <form className="chat-composer">
        <label className="sr-only" htmlFor="companion-message">
          Message Companion
        </label>
        <textarea
          id="companion-message"
          name="message"
          rows={1}
          placeholder="Message Companion…"
          aria-describedby="composer-note"
        />
        <div className="chat-composer__toolbar">
          <button className="composer-button" type="button" aria-label="Attach context">
            <AttachIcon />
          </button>
          <span className="chat-composer__model">Companion</span>
          <button className="composer-send" type="button" aria-label="Send message">
            <SendIcon />
          </button>
        </div>
      </form>
      <p className="composer-note" id="composer-note">
        Your conversations stay in your private workspace.
      </p>
    </main>
  );
}
