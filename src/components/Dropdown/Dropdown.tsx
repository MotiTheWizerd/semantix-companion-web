// Migrated from the studio's shared Dropdown (semantix-indexer/studio):
// portaled, viewport-clamped, searchable, keyboard-navigable. lucide-react is
// not a dependency here, so the chevron is a local inline SVG.

import {
  useState,
  useRef,
  useEffect,
  useLayoutEffect,
  useCallback,
} from "react";
import { createPortal } from "react-dom";
import { useDismiss } from "../../lib/useDismiss";
import styles from "./Dropdown.module.css";

/** Gap between trigger and menu, and the margin the menu keeps off the viewport
 *  edges. A menu shorter than MENU_MIN_H isn't worth flipping for. */
const GAP = 4;
const VIEWPORT_PAD = 8;
const MENU_MIN_H = 120;

/** Open dropdowns, bottom-up. Only the TOPMOST owns the document keyboard
 *  listeners — a dropdown nested inside another's menu would otherwise make
 *  one Enter select in BOTH. */
const openStack: symbol[] = [];

function Chevron({ className }: { className?: string }) {
  return (
    <svg
      className={className}
      viewBox="0 0 24 24"
      width={11}
      height={11}
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

interface MenuCoords {
  left: number;
  /** Exactly one of top/bottom is a number; the other is 'auto'. */
  top: number | "auto";
  bottom: number | "auto";
  maxHeight: number;
  up: boolean;
}

export interface DropdownProps<T> {
  items: T[];
  value: T | null;
  onChange: (item: T) => void;
  renderItem: (item: T) => React.ReactNode;
  renderTrigger?: (isOpen: boolean, selectedItem: T | null) => React.ReactNode;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  getItemKey: (item: T) => string;
  direction?: "up" | "down";
  searchable?: boolean;
  searchPlaceholder?: string;
  getSearchText?: (item: T) => string;
  /** Optional content pinned to the top of the open menu, above the search
   *  input (e.g. a brand/legend row). Renders as-given — the caller owns its
   *  styling. */
  menuHeader?: React.ReactNode;
  /** Optional content between the search input and the item list. */
  menuSubheader?: React.ReactNode;
  /** Extra class on the popup menu — lets a caller widen/tune just its own
   *  menu without touching the shared default. */
  menuClassName?: string;
  /** Extra classes on the trigger button — scoped restyles, same contract as
   *  menuClassName. */
  triggerClassName?: string;
}

export function Dropdown<T>({
  items,
  value,
  onChange,
  renderItem,
  renderTrigger,
  placeholder = "Select...",
  disabled = false,
  className = "",
  getItemKey,
  direction = "down",
  searchable = false,
  searchPlaceholder = "Search...",
  getSearchText,
  menuHeader,
  menuSubheader,
  menuClassName = "",
  triggerClassName = "",
}: DropdownProps<T>) {
  const [isOpen, setIsOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [highlightedIndex, setHighlightedIndex] = useState(-1);
  const [coords, setCoords] = useState<MenuCoords | null>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const itemsContainerRef = useRef<HTMLDivElement>(null);
  /** This instance's identity on the open-dropdown stack. */
  const stackIdRef = useRef(Symbol("dropdown"));

  useEffect(() => {
    if (!isOpen) return;
    const id = stackIdRef.current;
    openStack.push(id);
    return () => {
      const index = openStack.indexOf(id);
      if (index !== -1) openStack.splice(index, 1);
    };
  }, [isOpen]);

  const filteredItems =
    searchable && searchQuery
      ? items.filter((item) => {
          const searchText = getSearchText
            ? getSearchText(item)
            : String(renderItem(item));
          return searchText.toLowerCase().includes(searchQuery.toLowerCase());
        })
      : items;

  const handleSelect = useCallback(
    (item: T) => {
      onChange(item);
      setIsOpen(false);
    },
    [onChange],
  );

  const toggleDropdown = () => {
    if (!disabled) setIsOpen(!isOpen);
  };

  // Close on click outside / Escape. The menu is PORTALED to <body>, so it
  // lives outside dropdownRef — a plain contains() check would read a click on
  // an item as "outside" and dismiss before its onClick fires.
  useDismiss({
    open: isOpen,
    ref: dropdownRef,
    onDismiss: () => setIsOpen(false),
    ignore: "[data-dropdown-menu]",
  });

  /**
   * Place the portaled menu in VIEWPORT coordinates, measured from the
   * trigger. `direction` stays the caller's PREFERENCE — honored whenever the
   * menu fits — but a side with no room loses to the side that has some.
   */
  const position = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();

    let left = rect.left;
    const menuWidth = menuRef.current?.offsetWidth ?? 0;
    if (menuWidth && left + menuWidth > window.innerWidth - VIEWPORT_PAD) {
      left = window.innerWidth - menuWidth - VIEWPORT_PAD;
    }
    if (left < VIEWPORT_PAD) left = VIEWPORT_PAD;

    const spaceBelow = window.innerHeight - rect.bottom - GAP - VIEWPORT_PAD;
    const spaceAbove = rect.top - GAP - VIEWPORT_PAD;
    const preferUp = direction === "up";
    const up = preferUp
      ? !(spaceAbove < MENU_MIN_H && spaceBelow > spaceAbove)
      : spaceBelow < MENU_MIN_H && spaceAbove > spaceBelow;

    setCoords({
      left,
      top: up ? "auto" : rect.bottom + GAP,
      bottom: up ? window.innerHeight - rect.top + GAP : "auto",
      maxHeight: Math.max(MENU_MIN_H, up ? spaceAbove : spaceBelow),
      up,
    });
  }, [direction]);

  // Measure once the menu is in the DOM but BEFORE paint, so the first frame
  // is already placed. Stays aligned if an ancestor scrolls or the window
  // resizes.
  useLayoutEffect(() => {
    if (!isOpen) {
      setCoords(null);
      return;
    }
    position();
    const recompute = () => position();
    window.addEventListener("scroll", recompute, true);
    window.addEventListener("resize", recompute);
    return () => {
      window.removeEventListener("scroll", recompute, true);
      window.removeEventListener("resize", recompute);
    };
  }, [isOpen, position]);

  // Focus search input + highlight the currently selected item on open.
  useEffect(() => {
    if (!isOpen) return;
    if (searchable && searchInputRef.current) {
      searchInputRef.current.focus();
    }
    const selectedIdx = value
      ? filteredItems.findIndex((item) => getItemKey(item) === getItemKey(value))
      : -1;
    setHighlightedIndex(selectedIdx);
    if (selectedIdx >= 0 && itemsContainerRef.current) {
      const el = itemsContainerRef.current.children[selectedIdx] as
        | HTMLElement
        | undefined;
      el?.scrollIntoView({ block: "center", behavior: "auto" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen, searchable]);

  // While searching, keep the first match highlighted so Enter selects it.
  useEffect(() => {
    if (!isOpen || !searchable || !searchQuery) return;
    setHighlightedIndex(filteredItems.length > 0 ? 0 : -1);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchQuery, isOpen, searchable]);

  // Keyboard navigation — only the topmost open dropdown answers.
  useEffect(() => {
    if (!isOpen) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (openStack[openStack.length - 1] !== stackIdRef.current) return;
      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          setHighlightedIndex((prev) =>
            prev < filteredItems.length - 1 ? prev + 1 : prev,
          );
          break;
        case "ArrowUp":
          event.preventDefault();
          setHighlightedIndex((prev) => (prev > 0 ? prev - 1 : 0));
          break;
        case "Enter":
          event.preventDefault();
          if (highlightedIndex >= 0 && highlightedIndex < filteredItems.length) {
            handleSelect(filteredItems[highlightedIndex]);
          }
          break;
      }
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, highlightedIndex, filteredItems, handleSelect]);

  // Scroll the highlighted item into view (keyboard nav).
  useEffect(() => {
    if (highlightedIndex >= 0 && itemsContainerRef.current) {
      const highlighted = itemsContainerRef.current.children[
        highlightedIndex
      ] as HTMLElement | undefined;
      highlighted?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    }
  }, [highlightedIndex]);

  return (
    <div
      ref={dropdownRef}
      className={`${styles.dropdown} ${className} ${disabled ? styles.disabled : ""}`}
      data-direction={direction}
    >
      <button
        ref={triggerRef}
        type="button"
        className={`${styles.trigger} ${isOpen ? styles.open : ""} ${triggerClassName}`}
        onClick={toggleDropdown}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={isOpen}
      >
        {renderTrigger ? (
          renderTrigger(isOpen, value)
        ) : (
          <>
            <span className={styles.triggerText}>
              {value ? renderItem(value) : placeholder}
            </span>
            <Chevron className={styles.chevron} />
          </>
        )}
      </button>

      {isOpen &&
        createPortal(
          // PORTALED to <body> with fixed positioning so no scrolling or
          // overflow-hidden ancestor can clip the menu.
          <div
            ref={menuRef}
            data-dropdown-menu
            className={`${styles.menu} ${coords?.up ? styles.menuUp : styles.menuDown} ${menuClassName}`}
            role="listbox"
            style={{
              position: "fixed",
              left: coords?.left ?? 0,
              top: coords?.top ?? 0,
              bottom: coords?.bottom ?? "auto",
              maxHeight: coords?.maxHeight,
              // Pre-measurement frame: mounted so it can be measured,
              // invisible so it is never seen at the wrong place.
              visibility: coords ? "visible" : "hidden",
            }}
          >
            {menuHeader}
            {searchable && (
              <div className={styles.searchContainer}>
                <input
                  ref={searchInputRef}
                  type="text"
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                  placeholder={searchPlaceholder}
                  className={styles.searchInput}
                  onClick={(event) => event.stopPropagation()}
                />
              </div>
            )}
            {menuSubheader}
            <div className={styles.itemsContainer} ref={itemsContainerRef}>
              {filteredItems.length === 0 ? (
                <div className={styles.empty}>
                  {searchQuery ? "No matches found" : "No items available"}
                </div>
              ) : (
                filteredItems.map((item, index) => {
                  const key = getItemKey(item);
                  const isSelected = !!value && getItemKey(value) === key;
                  const isHighlighted = index === highlightedIndex;

                  return (
                    <button
                      key={key}
                      type="button"
                      className={`${styles.item} ${isSelected ? styles.selected : ""} ${isHighlighted ? styles.highlighted : ""}`}
                      onClick={() => handleSelect(item)}
                      onMouseEnter={() => setHighlightedIndex(index)}
                      role="option"
                      aria-selected={isSelected}
                    >
                      {renderItem(item)}
                      {isSelected && <span className={styles.checkmark}>✓</span>}
                    </button>
                  );
                })
              )}
            </div>
          </div>,
          document.body,
        )}
    </div>
  );
}
