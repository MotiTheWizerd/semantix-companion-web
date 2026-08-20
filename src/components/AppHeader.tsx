function MoreIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <circle cx="4.5" cy="10" r="1" />
      <circle cx="10" cy="10" r="1" />
      <circle cx="15.5" cy="10" r="1" />
    </svg>
  );
}

interface AppHeaderProps {
  eyebrow: string;
  title: string;
  showOptions?: boolean;
}

export function AppHeader({ eyebrow, title, showOptions = false }: AppHeaderProps) {
  return (
    <header className="app-header">
      <div className="app-header__title">
        <span className="app-header__eyebrow">{eyebrow}</span>
        <strong>{title}</strong>
      </div>
      <div className="app-header__actions">
        <span className="privacy-status">
          <span className="privacy-status__dot" />
          Private
        </span>
        {showOptions ? (
          <button className="icon-button" type="button" aria-label="Conversation options">
            <MoreIcon />
          </button>
        ) : null}
      </div>
    </header>
  );
}
