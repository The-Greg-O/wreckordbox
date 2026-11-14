/**
 * The command palette (ticket #54, ADR 0007).
 *
 * Lists every entry of the registry (`src/lib/palette.ts`) with its key,
 * narrows the list as the DJ types, and runs the chosen entry the way its
 * menu item, key or button would. An entry that cannot run now stays in the
 * list, greyed, with the reason. On Linux there is no native menu row, so
 * this is where the menus' items are.
 *
 * `CommandPaletteHost` is drawn once by the window, in both layouts. It owns
 * the palette's keys: its own opener on every platform, and on Linux the
 * menu's accelerators that no other handler answers (Import Folder, Full
 * screen, Close Window, Quit). `CommandPaletteButton` opens it with the
 * pointer, and `openCommandPalette` from anywhere else.
 */
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { flushSync } from "react-dom";

import { getBackend } from "@/ipc/client";
import { useTranslation } from "@/i18n";
import { browserFocus, setBrowserFocus, type BrowserFocus } from "@/lib/browserFocus";
import { hasCommand, runCommand } from "@/lib/commands";
import { hasEditHistory } from "@/lib/editHistory";
import { menuBarEnabled, menuBarSide, menuBarState } from "@/lib/menuBar";
import {
  PALETTE_ENTRIES, PALETTE_OPEN_CHORD, PALETTE_SURFACE, filterPalette, paletteAvailability, paletteChord, paletteEntry,
  paletteKey, type PaletteContext, type PaletteEntry,
} from "@/lib/palette";
import { describeChord, isTyping, type KeyOverrides, type Platform } from "@/lib/shortcuts";
import { useEventCallback } from "@/store/useEventCallback";
import type { PreferencesTarget } from "@/views/settings/Preferences";
import styles from "./CommandPalette.module.css";

const openers = new Set<() => void>();

/** Opens the palette, if the window has drawn it and nothing modal owns the keyboard. */
export function openCommandPalette(): void {
  for (const open of [...openers]) open();
}

/**
 * Whether a modal dialog owns the keyboard: a native `<dialog>` opened with
 * `showModal()` (the browser's top layer, which nothing drawn in the page
 * can sit above or take the focus from), or an ARIA `dialog` /
 * `alertdialog` that says it is modal, or the press itself came from inside
 * any dialog. The palette does not open over one: it would be drawn under
 * it, with its field out of reach.
 */
export function modalOwnsInput(target: EventTarget | null = null): boolean {
  if (typeof document === "undefined") return false;
  if (target instanceof Element && target.closest('dialog, [role="dialog"], [role="alertdialog"]') !== null &&
      target.closest(`[${PALETTE_SURFACE}]`) === null) {
    return true;
  }
  let nativeModal: boolean;
  try {
    nativeModal = document.querySelector("dialog:modal") !== null;
  } catch {
    // An engine without `:modal`: any open native dialog counts.
    nativeModal = document.querySelector("dialog[open]") !== null;
  }
  if (nativeModal) return true;
  for (const element of document.querySelectorAll('[role="alertdialog"], [role="dialog"][aria-modal="true"]')) {
    if (element.closest(`[${PALETTE_SURFACE}]`) !== null) continue;
    if (element.getClientRects().length > 0) return true;
  }
  return false;
}

export interface CommandPaletteHostProps {
  platform: Platform;
  /** The DJ's own keys, as every key handler reads them. */
  keyOverrides: KeyOverrides;
  /** The view shell is drawn, so the old layout's parts are not. */
  shell: boolean;
  /** The Library is locked for editing (rekordbox, or Library Protection). */
  readOnly: boolean;
  /** Whether the Library's own history has an edit to undo or redo. */
  history: { undo: boolean; redo: boolean };
  /** Whether each deck holds a track. */
  loaded: { a: boolean; b: boolean };
  /** The window's menu handler: what a native menu item's click ran. */
  runMenu: (id: string) => void;
  /** From the view shell: switches to the old layout and shows the part a menu item names. */
  runOldLayout: (id: string) => void;
  /** Shows the view shell (true) or the old layout (false). */
  setShell: (on: boolean) => void;
  openPreferences: (pane: PreferencesTarget) => void;
}

/** What had the keyboard when the palette opened, given back whenever it closes. */
interface Snapshot {
  focused: Element | null;
  /** A text field's selection, kept so Paste and Cut act on what was selected. */
  selection: { start: number; end: number; direction: "forward" | "backward" | "none" } | null;
  /** A selection anywhere else (a contenteditable), as a range. */
  range: Range | null;
  browser: BrowserFocus;
}

function snapshot(): Snapshot {
  const focused = typeof document === "undefined" ? null : document.activeElement;
  let selection: Snapshot["selection"] = null;
  if ((focused instanceof HTMLInputElement || focused instanceof HTMLTextAreaElement) && focused.selectionStart !== null) {
    selection = {
      start: focused.selectionStart,
      end: focused.selectionEnd ?? focused.selectionStart,
      direction: focused.selectionDirection ?? "none",
    };
  }
  const current = typeof window === "undefined" ? null : window.getSelection();
  const range = selection === null && current !== null && current.rangeCount > 0 ? current.getRangeAt(0).cloneRange() : null;
  return { focused, selection, range, browser: browserFocus() };
}

/**
 * Gives the keyboard back to what had it: the element, its selection, and
 * the browser's list or tree. An element that is gone, or the page itself,
 * leaves nothing focused (never the palette's field, which is gone too).
 */
function restore(before: Snapshot): void {
  const target = before.focused;
  if (target instanceof HTMLElement && target.isConnected && target !== document.body) {
    target.focus({ preventScroll: true });
    if (before.selection !== null && (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement)) {
      try {
        target.setSelectionRange(before.selection.start, before.selection.end, before.selection.direction);
      } catch {
        // A field type with no selection (a number field): nothing to keep.
      }
    } else if (before.range !== null) {
      const selection = window.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(before.range);
    }
  } else {
    const active = document.activeElement;
    if (active instanceof HTMLElement && active !== document.body) active.blur();
  }
  setBrowserFocus(before.browser);
}

export function CommandPaletteHost({
  platform, keyOverrides, shell, readOnly, history, loaded, runMenu, runOldLayout, setShell, openPreferences,
}: CommandPaletteHostProps) {
  const [opened, setOpened] = useState<Snapshot | null>(null);
  /** Bumped to redraw the list when an entry turned out unavailable as it was chosen. */
  const [, setChecked] = useState(0);

  /** What the window is like now, seen from where the DJ was when the palette opened. */
  const contextFor = (before: Snapshot): PaletteContext => {
    const focus = before.browser;
    const typing = isTyping(before.focused);
    return {
      shell,
      readOnly,
      enabled: menuBarEnabled({
        focus,
        ...menuBarState(),
        has: (action, side) => (side === undefined ? hasCommand(action) : hasCommand(action, side)),
      }),
      has: (action, scope) => hasCommand(action, scope),
      side: menuBarSide(focus),
      // A text field keeps its own history; a deck's grid editor its own.
      history: {
        undo: typing || hasEditHistory("undo") || history.undo,
        redo: typing || hasEditHistory("redo") || history.redo,
      },
      loaded,
    };
  };

  const open = useEventCallback(() => {
    if (modalOwnsInput()) return;
    setOpened((current) => current ?? snapshot());
  });
  useEffect(() => {
    openers.add(open);
    return () => {
      openers.delete(open);
    };
  }, [open]);

  /** Runs an entry: what its menu item, key or button would have done. */
  const run = useEventCallback((entry: PaletteEntry, before: Snapshot | null) => {
    const action = entry.run;
    const side = menuBarSide(before?.browser ?? browserFocus());
    switch (action.kind) {
      case "menu":
        if (entry.oldLayoutOnly === true && shell) runOldLayout(action.id);
        else runMenu(action.id);
        return;
      case "shell":
        void getBackend().then((backend) => backend.runMenuItem(action.id)).catch(() => undefined);
        return;
      case "command":
        if (action.scope === undefined) runCommand(action.action);
        else runCommand(action.action, { scope: action.scope === "browser" ? side : action.scope });
        return;
      case "layout":
        setShell(action.to === "shell");
        return;
      case "preferences":
        openPreferences(action.pane as PreferencesTarget);
        return;
      case "edit": {
        if (action.edit === "selectAll" && !isTyping(before?.focused)) {
          runCommand("selectAll", { scope: side });
          return;
        }
        // The webview's own editing action, on the field that has the
        // keyboard back by now, with its selection: the privileged path the
        // native Edit menu took (JavaScript's clipboard is off in WebKitGTK).
        const edit = action.edit;
        void getBackend()
          .then((backend) => backend.runEditCommand(edit))
          .catch(() => {
            // No native path on this platform (it has its menu row there):
            // the page's own, which can at least cut, copy and select.
            document.execCommand(edit);
          });
        return;
      }
    }
  });

  /** Closes the palette, now, and gives the keyboard back. */
  const dismiss = (before: Snapshot) => {
    // Unmounted before anything else happens, so its field is no longer the
    // focus a command looks at (Undo would otherwise act on its text).
    flushSync(() => setOpened(null));
    restore(before);
  };

  // The palette's keys, heard on the way down so that the opener works
  // wherever the keyboard is. While the palette is open its own field has
  // the keys; while a modal dialog has them, the palette stays shut.
  const onKey = useEventCallback((event: KeyboardEvent) => {
    if (opened !== null || event.defaultPrevented) return;
    const asked = paletteKey(event, platform, keyOverrides);
    if (asked === null) return;
    if (modalOwnsInput(event.target)) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) return;
    if (asked === "open") {
      open();
      return;
    }
    const entry = paletteEntry(asked);
    if (entry !== null) run(entry, null);
  });
  useEffect(() => {
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onKey]);

  if (opened === null) return null;
  return (
    <CommandPalette
      platform={platform}
      keyOverrides={keyOverrides}
      context={contextFor(opened)}
      onClose={() => dismiss(opened)}
      onRun={(entry) => {
        // Checked again as it runs: the window may have changed since the
        // list was drawn (the Library locked, the deck emptied).
        if (!paletteAvailability(entry, contextFor(opened)).available) {
          setChecked((count) => count + 1);
          return;
        }
        dismiss(opened);
        run(entry, opened);
      }}
    />
  );
}

interface CommandPaletteProps {
  platform: Platform;
  keyOverrides: KeyOverrides;
  context: PaletteContext;
  onClose: () => void;
  onRun: (entry: PaletteEntry) => void;
}

function CommandPalette({ platform, keyOverrides, context, onClose, onRun }: CommandPaletteProps) {
  const t = useTranslation();
  const listId = useId();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const field = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);

  const found = useMemo(() => filterPalette(PALETTE_ENTRIES, query, t), [query, t]);
  const shown = found.map((entry) => ({
    entry,
    availability: paletteAvailability(entry, context),
    chord: paletteChord(entry, keyOverrides),
  }));
  const current = Math.min(active, Math.max(shown.length - 1, 0));
  const optionId = (index: number) => `${listId}-${index}`;

  useLayoutEffect(() => {
    field.current?.focus();
  }, []);
  useLayoutEffect(() => {
    list.current?.querySelector(`[data-index="${current}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [current]);

  const choose = (index: number) => {
    const row = shown[index];
    if (row === undefined || !row.availability.available) return;
    onRun(row.entry);
  };

  const onKeyDown = (event: ReactKeyboardEvent) => {
    // Nothing typed here reaches the window's own keys: the list, the deck
    // and the view keys stay where they were.
    event.stopPropagation();
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        setActive(shown.length === 0 ? 0 : (current + 1) % shown.length);
        return;
      case "ArrowUp":
        event.preventDefault();
        setActive(shown.length === 0 ? 0 : (current - 1 + shown.length) % shown.length);
        return;
      case "PageDown":
      case "End":
        if (event.key === "End" && !event.ctrlKey) return;
        event.preventDefault();
        setActive(Math.max(shown.length - 1, 0));
        return;
      case "PageUp":
      case "Home":
        if (event.key === "Home" && !event.ctrlKey) return;
        event.preventDefault();
        setActive(0);
        return;
      case "Enter":
        event.preventDefault();
        choose(current);
        return;
      case "Escape":
        event.preventDefault();
        onClose();
        return;
      case "Tab":
        // The field is the palette's only stop.
        event.preventDefault();
        return;
      default:
    }
  };

  return (
    <div
      className={styles.backdrop}
      data-testid="command-palette"
      {...{ [PALETTE_SURFACE]: "" }}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className={styles.palette} role="dialog" aria-modal="true" aria-label={t("Command palette")} onKeyDown={onKeyDown}>
        <input
          ref={field}
          className={styles.field}
          type="text"
          role="combobox"
          aria-label={t("Search actions")}
          aria-expanded="true"
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={shown.length > 0 ? optionId(current) : undefined}
          placeholder={t("Type an action")}
          spellCheck={false}
          autoComplete="off"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(0);
          }}
        />
        <ul ref={list} id={listId} className={styles.list} role="listbox" aria-label={t("Actions")}>
          {shown.map(({ entry, availability, chord }, index) => (
            <li
              key={entry.id}
              id={optionId(index)}
              data-index={index}
              data-entry={entry.id}
              className={styles.option}
              role="option"
              aria-selected={index === current}
              aria-disabled={availability.available ? undefined : true}
              onMouseDown={(event) => event.preventDefault()}
              onMouseMove={() => {
                if (index !== current) setActive(index);
              }}
              onClick={() => choose(index)}
            >
              <span className={styles.label}>{t(entry.label)}</span>
              <span className={styles.group}>{t(entry.group)}</span>
              {availability.available
                ? availability.note === undefined ? null : <span className={styles.why}>{t(availability.note)}</span>
                : <span className={styles.why}>{t(availability.why)}</span>}
              {chord !== null ? <kbd className={styles.key}>{describeChord(chord, platform)}</kbd> : null}
            </li>
          ))}
        </ul>
        {shown.length === 0 ? <p className={styles.empty}>{t("No action matches")}</p> : null}
      </div>
    </div>
  );
}

/** Opens the palette with the pointer, showing its key. */
export function CommandPaletteButton({ platform, className }: { platform: Platform; className?: string }) {
  const t = useTranslation();
  const key = describeChord(PALETTE_OPEN_CHORD, platform);
  return (
    <button
      type="button"
      className={className === undefined ? styles.opener : `${styles.opener} ${className}`}
      aria-label={t("Command palette")}
      aria-keyshortcuts={key}
      title={t("Command palette")}
      onClick={() => openCommandPalette()}
    >
      {/* At the shell's narrowest (wbx-45.22) a prompt glyph stands in for the word; the name stays. */}
      <svg className={styles.openerGlyph} viewBox="0 0 12 10" aria-hidden>
        <path d="M1 1.5 4.5 5 1 8.5M6 8.5h5" stroke="currentColor" strokeWidth="1.3" fill="none" />
      </svg>
      <span className={styles.openerWord}>{t("Commands")}</span>
      <kbd className={styles.key}>{key}</kbd>
    </button>
  );
}
