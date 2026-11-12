/**
 * A right-click menu, drawn as rekordbox draws its own.
 *
 * The rows come in as data — see `src/lib/contextMenus.ts`, which transcribes
 * rekordbox's lists — so this only decides where the menu goes, what closes
 * it, and how a greyed row differs from a live one.
 */
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { enabled, SEPARATOR, type MenuContext, type MenuRow } from "@/lib/contextMenus";
import styles from "./ContextMenu.module.css";

/** Kept off the window's edges, so a menu opened near one is still readable. */
const EDGE = 8;

export interface ContextMenuProps<A extends string> {
  x: number;
  y: number;
  rows: readonly MenuRow<A>[];
  context: MenuContext;
  /** Named for the screen reader, since a menu with no name is "menu". */
  label: string;
  /**
   * A heading drawn over the entries, above a rule: rekordbox's "File is
   * Missing" over a missing track's menu. Most menus have none.
   */
  title?: string | undefined;
  onChoose: (action: A) => void;
  onClose: () => void;
  /**
   * Worked from the keyboard (the shell's tree, wbx-45.4): the first live
   * entry takes the focus when the menu opens, Up and Down (and Home, End)
   * move between live entries, Right opens a submenu and Left closes it,
   * Enter or Space chooses, Escape closes. Off, the menu is as it was.
   */
  keyboard?: boolean;
}

/** The live entries of one menu level, top to bottom. */
function liveItems(menu: Element | null): HTMLButtonElement[] {
  if (menu === null) return [];
  // The top level wraps each entry in a row; a submenu holds its entries directly.
  return [...menu.querySelectorAll<HTMLButtonElement>(
    ":scope > div > button[role^='menuitem']:not(:disabled), :scope > button[role^='menuitem']:not(:disabled)",
  )];
}

export function ContextMenu<A extends string>({
  x, y, rows, context, label, title, onChoose, onClose, keyboard = false,
}: ContextMenuProps<A>) {
  const box = useRef<HTMLDivElement>(null);
  const submenu = useRef<HTMLDivElement>(null);
  /** Which entry's submenu is open, by label. One at a time, as menus are. */
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    // Anything outside, any scroll, any resize, or Escape closes it: a menu
    // that outlives the thing it was opened on is worse than no menu.
    const outside = (event: MouseEvent) => {
      if (!box.current?.contains(event.target as Node)) onClose();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", outside, true);
    window.addEventListener("keydown", key);
    window.addEventListener("resize", onClose);
    const scroll = (event: Event) => {
      if (!box.current?.contains(event.target as Node)) onClose();
    };
    window.addEventListener("scroll", scroll, true);
    return () => {
      window.removeEventListener("mousedown", outside, true);
      window.removeEventListener("keydown", key);
      window.removeEventListener("resize", onClose);
      window.removeEventListener("scroll", scroll, true);
    };
  }, [onClose]);

  // Placed before the first paint, from its own measured size: a menu that is
  // one row shorter than the last is a different height, and guessing it puts
  // the bottom of a long menu under the status bar.
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    const { width, height } = element.getBoundingClientRect();
    const left = Math.min(x, Math.max(EDGE, window.innerWidth - width - EDGE));
    const top = Math.min(y, Math.max(EDGE, window.innerHeight - height - EDGE));
    element.style.left = `${Math.max(EDGE, left)}px`;
    element.style.top = `${Math.max(EDGE, top)}px`;
  }, [x, y, rows]);

  useLayoutEffect(() => {
    const element = submenu.current;
    const anchor = element?.parentElement;
    if (!element || !anchor) return;
    const row = anchor.getBoundingClientRect();
    const { width, height } = element.getBoundingClientRect();
    const right = row.right - 1;
    const left = right + width <= window.innerWidth - EDGE ? right : row.left - width + 1;
    element.style.left = `${Math.max(EDGE, Math.min(left, window.innerWidth - width - EDGE))}px`;
    element.style.top = `${Math.max(EDGE, Math.min(row.top, window.innerHeight - height - EDGE))}px`;
  }, [open, x, y, rows]);

  useEffect(() => {
    if (keyboard) liveItems(box.current)[0]?.focus();
  }, [keyboard]);
  // A submenu opened from the keyboard takes the focus too.
  const openedByKey = useRef(false);
  useEffect(() => {
    if (!keyboard || !openedByKey.current || open === null) return;
    openedByKey.current = false;
    liveItems(submenu.current)[0]?.focus();
  }, [keyboard, open]);

  const onKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!keyboard) return;
    const at = document.activeElement as HTMLElement | null;
    const inSub = submenu.current?.contains(at) === true;
    const items = liveItems(inSub ? submenu.current : box.current);
    const index = items.findIndex((item) => item === at);
    const move = (to: number) => items[(to + items.length) % items.length]?.focus();
    switch (event.key) {
      case "ArrowDown": move(index + 1); break;
      case "ArrowUp": move(index < 0 ? items.length - 1 : index - 1); break;
      case "Home": move(0); break;
      case "End": move(items.length - 1); break;
      case "ArrowRight":
        if (!inSub && at?.getAttribute("aria-haspopup") === "menu") {
          openedByKey.current = true;
          at.click();
        }
        break;
      case "ArrowLeft":
        if (inSub) {
          const parent = submenu.current?.parentElement?.querySelector<HTMLButtonElement>(":scope > button");
          setOpen(null);
          parent?.focus();
        }
        break;
      case "Escape": onClose(); break;
      case "Enter":
      case " ":
        // The focused entry's own click.
        return;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  };

  return createPortal(
    <div
      ref={box}
      className={styles.menu}
      style={{ left: x, top: y }}
      role="menu"
      aria-label={label}
      onKeyDown={onKey}
    >
      {title !== undefined ? (
        <>
          <div className={styles.title} role="presentation">{title}</div>
          <div className={styles.separator} role="separator" />
        </>
      ) : null}
      {rows.map((row, index) =>
        row === SEPARATOR ? (
          // Its position is the only thing a separator has to be keyed by, and
          // the rows are a constant: they never reorder under it.
          <div key={`sep-${index}`} className={styles.separator} role="separator" />
        ) : (
          <div
            key={row.label}
            className={styles.row}
            // Hover opens it and moving to another entry closes it, which is
            // what a menu does; the click is for a pointer that arrives
            // without hovering, and for the keyboard.
            onMouseEnter={() => setOpen(row.items ? row.label : null)}
          >
            <button
              type="button"
              role="menuitem"
              className={styles.item}
              disabled={!enabled(row, context)}
              // Said as well as done: a greyed entry is still read out as one
              // rekordbox has, just one this cannot choose.
              aria-disabled={enabled(row, context) ? undefined : true}
              aria-haspopup={row.items ? "menu" : undefined}
              aria-expanded={row.items ? open === row.label : undefined}
              onClick={() => {
                if (row.items) {
                  setOpen((was) => (was === row.label ? null : row.label));
                  return;
                }
                if (row.action !== null) onChoose(row.action);
                onClose();
              }}
            >
              <span className={styles.label}>{row.label}</span>
              {row.submenu === true || row.items ? (
                <span className={styles.arrow} aria-hidden />
              ) : null}
            </button>
            {row.items && open === row.label ? (
              <div ref={submenu} className={styles.submenu} role="menu" aria-label={row.label}>
                {row.items.map((child, at) =>
                  child === SEPARATOR ? (
                    <div key={`sub-${at}`} className={styles.separator} role="separator" />
                  ) : (
                    <button
                      key={child.label}
                      type="button"
                      role={child.checked === undefined ? "menuitem" : "menuitemradio"}
                      aria-checked={child.checked}
                      className={styles.item}
                      data-checked={child.checked || undefined}
                      disabled={!enabled(child, context)}
                      aria-disabled={enabled(child, context) ? undefined : true}
                      onClick={() => {
                        if (child.action !== null) onChoose(child.action);
                        onClose();
                      }}
                    >
                      {child.checked ? <span className={styles.tick} aria-hidden /> : null}
                      <span className={styles.label}>{child.label}</span>
                    </button>
                  ),
                )}
              </div>
            ) : null}
          </div>
        ),
      )}
    </div>,
    document.body,
  );
}
