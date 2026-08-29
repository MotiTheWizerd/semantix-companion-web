import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import { useDismiss } from "../../lib/useDismiss";
import {
  ALL_EMOJIS,
  EMOJI_BY_CHARACTER,
  EMOJI_CATEGORIES,
  emojiSearchText,
  type EmojiEntry,
} from "./emojiData";
import styles from "./EmojiPicker.module.css";

const VIEWPORT_PAD = 8;
const PANEL_GAP = 7;
const PANEL_WIDTH = 336;
const PANEL_MAX_HEIGHT = 410;
const PANEL_MIN_HEIGHT = 220;
const DEFAULT_STORAGE_KEY = "companion.emoji.recents";

interface PanelPosition {
  left: number;
  top: number | "auto";
  bottom: number | "auto";
  width: number;
  maxHeight: number;
  up: boolean;
}

export interface EmojiPickerProps {
  onSelect: (emoji: string) => void;
  disabled?: boolean;
  className?: string;
  triggerClassName?: string;
  ariaLabel?: string;
  direction?: "up" | "down";
  storageKey?: string;
  recentLimit?: number;
  returnFocus?: boolean;
  renderTrigger?: (isOpen: boolean) => ReactNode;
}

function SmileIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <circle cx="10" cy="10" r="6.75" />
      <path d="M7.2 11.15c.65 1.15 1.57 1.72 2.8 1.72s2.15-.57 2.8-1.72" />
      <path d="M7.6 7.9h.01M12.4 7.9h.01" />
    </svg>
  );
}

function SearchIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <circle cx="8.75" cy="8.75" r="4.75" />
      <path d="m12.25 12.25 3.25 3.25" />
    </svg>
  );
}

function readRecents(storageKey: string, limit: number): string[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(storageKey) ?? "[]") as unknown;
    if (!Array.isArray(parsed)) return [];
    return [...new Set(parsed)]
      .filter(
        (value): value is string =>
          typeof value === "string" && EMOJI_BY_CHARACTER.has(value),
      )
      .slice(0, limit);
  } catch {
    return [];
  }
}

function writeRecents(storageKey: string, emojis: readonly string[]): void {
  try {
    localStorage.setItem(storageKey, JSON.stringify(emojis));
  } catch {
    // Storage can be unavailable in a hardened webview. The live picker still
    // remembers for this mount; only cross-session persistence stands down.
  }
}

function EmojiGrid({
  entries,
  onSelect,
}: {
  entries: readonly EmojiEntry[];
  onSelect: (entry: EmojiEntry) => void;
}) {
  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (
      !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(
        event.key,
      )
    ) {
      return;
    }
    const buttons = Array.from(
      event.currentTarget.querySelectorAll<HTMLButtonElement>("[data-emoji-button]"),
    );
    const current = event.target as HTMLButtonElement;
    const index = buttons.indexOf(current);
    if (index < 0) return;
    const columns = getComputedStyle(event.currentTarget).gridTemplateColumns.split(" ").length;
    let nextIndex = index;
    if (event.key === "ArrowLeft") nextIndex = Math.max(0, index - 1);
    if (event.key === "ArrowRight") nextIndex = Math.min(buttons.length - 1, index + 1);
    if (event.key === "ArrowUp") nextIndex = Math.max(0, index - columns);
    if (event.key === "ArrowDown") {
      nextIndex = Math.min(buttons.length - 1, index + columns);
    }
    if (event.key === "Home") nextIndex = 0;
    if (event.key === "End") nextIndex = buttons.length - 1;
    event.preventDefault();
    buttons[nextIndex]?.focus();
  };

  return (
    <div className={styles.emojiGrid} onKeyDown={handleKeyDown}>
      {entries.map((entry, index) => (
        <button
          key={entry.emoji}
          type="button"
          data-emoji-button
          className={styles.emojiButton}
          title={entry.label}
          aria-label={entry.label}
          tabIndex={index === 0 ? 0 : -1}
          onClick={() => onSelect(entry)}
        >
          {entry.emoji}
        </button>
      ))}
    </div>
  );
}

export function EmojiPicker({
  onSelect,
  disabled = false,
  className = "",
  triggerClassName = "",
  ariaLabel = "Choose emoji",
  direction = "up",
  storageKey = DEFAULT_STORAGE_KEY,
  recentLimit = 24,
  returnFocus = true,
  renderTrigger,
}: EmojiPickerProps) {
  const safeRecentLimit = Math.min(Math.max(recentLimit, 1), 48);
  const [isOpen, setIsOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [recent, setRecent] = useState(() =>
    readRecents(storageKey, safeRecentLimit),
  );
  const [position, setPosition] = useState<PanelPosition | null>(null);
  const generatedId = useId();
  const panelId = `${generatedId}-emoji-panel`;
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setRecent(readRecents(storageKey, safeRecentLimit));
  }, [safeRecentLimit, storageKey]);

  useDismiss({
    open: isOpen,
    ref: rootRef,
    onDismiss: () => setIsOpen(false),
    ignore: "[data-emoji-picker-panel]",
  });

  const placePanel = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const width = Math.min(PANEL_WIDTH, window.innerWidth - VIEWPORT_PAD * 2);
    let left = rect.left;
    if (left + width > window.innerWidth - VIEWPORT_PAD) {
      left = window.innerWidth - width - VIEWPORT_PAD;
    }
    left = Math.max(VIEWPORT_PAD, left);

    const spaceAbove = rect.top - PANEL_GAP - VIEWPORT_PAD;
    const spaceBelow = window.innerHeight - rect.bottom - PANEL_GAP - VIEWPORT_PAD;
    const preferUp = direction === "up";
    const up = preferUp
      ? !(spaceAbove < PANEL_MIN_HEIGHT && spaceBelow > spaceAbove)
      : spaceBelow < PANEL_MIN_HEIGHT && spaceAbove > spaceBelow;
    const available = Math.max(0, up ? spaceAbove : spaceBelow);

    setPosition({
      left,
      top: up ? "auto" : rect.bottom + PANEL_GAP,
      bottom: up ? window.innerHeight - rect.top + PANEL_GAP : "auto",
      width,
      maxHeight: Math.min(PANEL_MAX_HEIGHT, available),
      up,
    });
  }, [direction]);

  useLayoutEffect(() => {
    if (!isOpen) {
      setPosition(null);
      return;
    }
    placePanel();
    window.addEventListener("scroll", placePanel, true);
    window.addEventListener("resize", placePanel);
    return () => {
      window.removeEventListener("scroll", placePanel, true);
      window.removeEventListener("resize", placePanel);
    };
  }, [isOpen, placePanel]);

  useEffect(() => {
    if (!isOpen) return;
    searchRef.current?.focus();
  }, [isOpen]);

  const normalizedQuery = query.trim().toLowerCase();
  const searchResults = useMemo(
    () =>
      normalizedQuery
        ? ALL_EMOJIS.filter((entry) =>
            emojiSearchText(entry).includes(normalizedQuery),
          )
        : [],
    [normalizedQuery],
  );
  const recentEntries = recent
    .map((emoji) => EMOJI_BY_CHARACTER.get(emoji))
    .filter((entry): entry is EmojiEntry => entry !== undefined);

  const choose = (entry: EmojiEntry) => {
    const next = [entry.emoji, ...recent.filter((emoji) => emoji !== entry.emoji)].slice(
      0,
      safeRecentLimit,
    );
    setRecent(next);
    writeRecents(storageKey, next);
    setIsOpen(false);
    onSelect(entry.emoji);
    if (returnFocus) {
      requestAnimationFrame(() => triggerRef.current?.focus());
    }
  };

  const toggle = () => {
    if (disabled) return;
    setIsOpen((open) => {
      if (!open) setQuery("");
      return !open;
    });
  };

  return (
    <div ref={rootRef} className={`${styles.picker} ${className}`}>
      <button
        ref={triggerRef}
        type="button"
        className={`${styles.trigger} ${isOpen ? styles.triggerOpen : ""} ${triggerClassName}`}
        aria-label={ariaLabel}
        aria-haspopup="dialog"
        aria-expanded={isOpen}
        aria-controls={isOpen ? panelId : undefined}
        disabled={disabled}
        onClick={toggle}
      >
        {renderTrigger ? renderTrigger(isOpen) : <SmileIcon />}
      </button>

      {isOpen &&
        createPortal(
          <section
            ref={panelRef}
            id={panelId}
            data-emoji-picker-panel
            className={`${styles.panel} ${position?.up ? styles.panelUp : styles.panelDown}`}
            role="dialog"
            aria-label="Emoji picker"
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                requestAnimationFrame(() => triggerRef.current?.focus());
              }
            }}
            style={{
              left: position?.left ?? 0,
              top: position?.top ?? 0,
              bottom: position?.bottom ?? "auto",
              width: position?.width,
              minHeight: position
                ? Math.min(PANEL_MIN_HEIGHT, position.maxHeight)
                : undefined,
              maxHeight: position?.maxHeight,
              visibility: position ? "visible" : "hidden",
            }}
          >
            <header className={styles.header}>
              <span>Emoji</span>
              <strong>Find the right reaction</strong>
            </header>
            <div className={styles.searchBox}>
              <span className={styles.searchIcon}>
                <SearchIcon />
              </span>
              <input
                ref={searchRef}
                type="search"
                value={query}
                placeholder="Search emoji..."
                aria-label="Search emoji"
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== "ArrowDown") return;
                  const firstEmoji =
                    panelRef.current?.querySelector<HTMLButtonElement>(
                      "[data-emoji-button]",
                    );
                  if (!firstEmoji) return;
                  event.preventDefault();
                  firstEmoji.focus();
                }}
              />
            </div>

            <div className={styles.scroller}>
              {normalizedQuery ? (
                <div className={styles.section}>
                  <div className={styles.sectionHeading}>
                    <span>Search results</span>
                    <small>{searchResults.length}</small>
                  </div>
                  {searchResults.length > 0 ? (
                    <EmojiGrid entries={searchResults} onSelect={choose} />
                  ) : (
                    <div className={styles.empty}>No emoji found for “{query.trim()}”</div>
                  )}
                </div>
              ) : (
                <>
                  <div className={styles.section}>
                    <div className={styles.sectionHeading}>
                      <span>Recent</span>
                      {recentEntries.length > 0 ? (
                        <small>{recentEntries.length}</small>
                      ) : null}
                    </div>
                    {recentEntries.length > 0 ? (
                      <EmojiGrid entries={recentEntries} onSelect={choose} />
                    ) : (
                      <div className={styles.emptyRecent}>
                        Your recently used emoji will appear here.
                      </div>
                    )}
                  </div>
                  {EMOJI_CATEGORIES.map((item) => (
                    <div className={styles.section} key={item.id}>
                      <div className={styles.sectionHeading}>
                        <span>
                          <i aria-hidden="true">{item.icon}</i>
                          {item.label}
                        </span>
                      </div>
                      <EmojiGrid entries={item.emojis} onSelect={choose} />
                    </div>
                  ))}
                </>
              )}
            </div>
          </section>,
          document.body,
        )}
    </div>
  );
}
