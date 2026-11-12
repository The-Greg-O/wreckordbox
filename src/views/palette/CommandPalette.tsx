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

import { getBackend } from "@/ipc/client";
import { useTranslation } from "@/i18n";
import { browserFocus, setBrowserFocus, type BrowserFocus } from "@/lib/browserFocus";
import { hasCommand, runCommand } from "@/lib/commands";
import { menuBarEnabled, menuBarSide, menuBarState } from "@/lib/menuBar";
import {
  PALETTE_ENTRIES, PALETTE_OPEN_CHORD, filterPalette, paletteAvailability, paletteChord, paletteEntry, paletteKey,
  type PaletteContext, type PaletteEntry,
} from "@/lib/palette";
import { describeChord, isTyping, type KeyOverrides, type Platform } from "@/lib/shortcuts";
import { useEventCallback } from "@/store/useEventCallback";
import type { PreferencesTarget } from "@/views/settings/Preferences";
import styles from "./CommandPalette.module.css";

const openers = new Set<() => void>();

/** Opens the palette, if the window has drawn it. */
export function openCommandPalette(): void {
  for (const open of [...openers]) open();
}

export interface CommandPaletteHostProps {
  platform: Platform;
  /** The DJ's own keys, as every key handler reads them. */
  keyOverrides: KeyOverrides;
  /** The view shell is drawn, so the old layout's parts are not. */
  shell: boolean;
  /** The Library is locked for editing. */
  readOnly: boolean;
  /** The window's menu handler: what a native menu item's click ran. */
  runMenu: (id: string) => void;
  openPreferences: (pane: PreferencesTarget) => void;
}

/** What the window was like when the palette opened, to run an entry against. */
interface Snapshot {
  context: PaletteContext;
  /** What had the keyboard, given back before an entry runs. */
  focused: Element | null;
  browser: BrowserFocus;
}

function snapshot(shell: boolean, readOnly: boolean): Snapshot {
  const focus = browserFocus();
  return {
    context: {
      shell,
      readOnly,
      enabled: menuBarEnabled({
        focus,
        ...menuBarState(),
        has: (action, side) => (side === undefined ? hasCommand(action) : hasCommand(action, side)),
      }),
      has: (action) => hasCommand(action),
    },
    focused: typeof document === "undefined" ? null : document.activeElement,
    browser: focus,
  };
}

export function CommandPaletteHost({
  platform, keyOverrides, shell, readOnly, runMenu, openPreferences,
}: CommandPaletteHostProps) {
  const [opened, setOpened] = useState<Snapshot | null>(null);

  const open = useEventCallback(() => {
    setOpened((current) => current ?? snapshot(shell, readOnly));
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
    switch (action.kind) {
      case "menu":
        runMenu(action.id);
        return;
      case "shell":
        void getBackend().then((backend) => backend.runMenuItem(action.id)).catch(() => undefined);
        return;
      case "command":
        runCommand(action.action);
        return;
      case "preferences":
        openPreferences(action.pane as PreferencesTarget);
        return;
      case "edit": {
        const typing = isTyping(before?.focused);
        if (action.edit === "selectAll" && !typing) {
          runCommand("selectAll", { scope: menuBarSide(before?.browser ?? browserFocus()) });
          return;
        }
        // The Edit menu's own items acted on the field that had the
        // keyboard; it has it back by now.
        document.execCommand(action.edit);
        return;
      }
    }
  });

  // The palette's keys, heard on the way down so that the opener works
  // wherever the keyboard is. While the palette is open its own field has
  // the keys.
  const onKey = useEventCallback((event: KeyboardEvent) => {
    if (opened !== null || event.defaultPrevented) return;
    const asked = paletteKey(event, platform, keyOverrides);
    if (asked === null) return;
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
      context={opened.context}
      onClose={() => {
        setOpened(null);
        (opened.focused as HTMLElement | null)?.focus?.();
      }}
      onRun={(entry) => {
        setOpened(null);
        // The entry acts on what the DJ was on before the palette took the
        // keyboard: the field, the list or the tree.
        (opened.focused as HTMLElement | null)?.focus?.();
        setBrowserFocus(opened.browser);
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

  const shown = useMemo(
    () => filterPalette(PALETTE_ENTRIES, query, t).map((entry) => ({
      entry,
      availability: paletteAvailability(entry, context),
      chord: paletteChord(entry, keyOverrides),
    })),
    [query, t, context, keyOverrides],
  );
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
              {availability.available ? null : <span className={styles.why}>{t(availability.why)}</span>}
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
      <span>{t("Commands")}</span>
      <kbd className={styles.key}>{key}</kbd>
    </button>
  );
}
