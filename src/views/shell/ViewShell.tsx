/**
 * The view shell: one frame, four views (ticket #50, ADR 0007).
 *
 * The frame is a top bar with the view tabs, a player area, a tree area, a
 * working area and a status line. Only the working area changes between
 * Library, Intake, Review and Stick; the tree, the player and the status
 * line are the same elements in every view, so nothing in them is torn down
 * by a switch. Playback, the loaded track and the tree's selection are held
 * by the window that renders the shell (`AppBody`) and handed in as the
 * `player`, `tree` and `status` areas' content.
 *
 * Where later tickets plug in:
 *  - `player`: the compact player (#51). Today, the old player at its smallest.
 *  - `tree`: the shell's tree (#52, `ShellTree.tsx`).
 *  - `status`: the status line's contents (#53). Today, the old status bar.
 *  - `topBarEnd`: the command palette's opener (#54), the waveform theme and
 *    the scale readout.
 *  - `library`: the Library view's list (#55 changes its columns).
 *  - `IntakeView`, `ReviewView`, `StickView`: their own files beside this one.
 *
 * The layout answers to the shell's own width through a container query (see
 * the stylesheet): at the breakpoint and below, the tree gives way.
 */
import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";

import { useTranslation } from "@/i18n";
import { runCommand } from "@/lib/commands";
import { describeChord, type KeyOverrides, type Platform } from "@/lib/shortcuts";
import { SHELL_VIEWS, shellViewChord, shellViewForKey, type ShellView } from "@/lib/shellViews";
import type { PlayerLayout } from "@/lib/layout";
import type { IntakeQueue } from "@/lib/shellTree";
import { useCommands } from "@/store/useCommands";
import { useEventCallback } from "@/store/useEventCallback";
import { observeShellWidth, useShellWidth } from "@/store/useShellWidth";
import { SHELL_NARROW_MAX_PX } from "@/lib/shellViews";
import { treeKeyAction } from "@/lib/shellTreeOverlay";
import { isTyping } from "@/lib/shortcuts";
import { LibraryView } from "./LibraryView";
import { ReviewView } from "./ReviewView";
import { StickView } from "./StickView";
import styles from "./ViewShell.module.css";

/**
 * The deck layout the shell's player area holds until the compact player
 * exists (ticket #51): one deck, at its smallest.
 */
export const SHELL_PLAYER_LAYOUT: PlayerLayout = "simple";

export interface ViewShellProps {
  /** The open list's breadcrumb (its folders, then its name), drawn in place of its heading at half width (wbx-45.21). */
  libraryCrumb?: readonly string[] | undefined;
  /** The view in the working area. Held by the window, so it can be asked for from outside the shell. */
  view: ShellView;
  onViewChange: (view: ShellView) => void;
  /** The tree area's content: the same in every view. */
  tree: ReactNode;
  /** The player area's content: the same in every view. */
  player: ReactNode;
  /** The status line's content: the same in every view. */
  status: ReactNode;
  /** The Library view's list. */
  library: ReactNode;
  /** The Library view's second list, the target playlist (wbx-59), when shown. */
  libraryTarget?: ReactNode;
  /** The right-hand end of the top bar. */
  topBarEnd?: ReactNode;
  platform: Platform;
  /** The DJ's own keys, as every key handler reads them. */
  keyOverrides?: KeyOverrides;
  /** The queue the Intake view shows, as chosen in the tree. */
  intakeQueue?: IntakeQueue | undefined;
  /** The Stick view's content: the Stick selected in the tree, if any. */
  stick?: ReactNode | undefined;
  /** The number waiting, drawn on a view's tab ("Intake 23"); absent or `null` draws none. */
  tabCounts?: Partial<Record<ShellView, number | null>> | undefined;
  /** The Intake view: given the queue it shows, so the shell stays out of its data. */
  intake?: ((queue: IntakeQueue) => ReactNode) | undefined;
}

function tabId(view: ShellView): string {
  return `shell-tab-${view}`;
}

export function ViewShell({
  view, onViewChange, tree, player, status, library, libraryTarget, libraryCrumb, topBarEnd, platform, keyOverrides, intakeQueue, stick, tabCounts, intake,
}: ViewShellProps) {
  const t = useTranslation();

  // Showing a view is a command, so a tab, a key and (later) the command
  // palette all ask for it the same way.
  useCommands({
    viewLibrary: () => { onViewChange("library"); return true; },
    viewIntake: () => { onViewChange("intake"); return true; },
    viewReview: () => { onViewChange("review"); return true; },
    viewStick: () => { onViewChange("stick"); return true; },
  });

  // The view keys. Heard on the way down (capture), ahead of the deck and
  // the list, because 1 to 4 are also the deck's Set Hot Cue A to C and
  // 1 Beat Loop in the Export key map: with the shell on those keys are the
  // views', so the press stops here. See `shellViews.ts`.
  const onKey = useEventCallback((event: KeyboardEvent) => {
    const asked = shellViewForKey(event, platform, event.target as HTMLElement | null, keyOverrides);
    if (asked === null) return;
    // A held view key is still the view's, not the deck's: it stops here too.
    const action = SHELL_VIEWS.find((entry) => entry.id === asked)?.action;
    if (action !== undefined && !event.repeat) runCommand(action);
    event.preventDefault();
    event.stopPropagation();
  });
  useEffect(() => {
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onKey]);

  // The tree when the shell is too narrow to draw it (wbx-45.21): "Go to
  // the Tree" (the Library's breadcrumb, the palette, Ctrl+T) draws it over
  // the view; choosing a row, Escape or a click elsewhere puts it away. It
  // is the same mounted tree, so its selection, folders and the target's
  // mark are what they were. At full width the command moves the keyboard
  // to it.
  const shellWidth = useShellWidth();
  const narrow = shellWidth !== null && shellWidth <= SHELL_NARROW_MAX_PX;
  const [treeOpen, setTreeOpen] = useState(false);
  const treeRef = useRef<HTMLElement>(null);
  useEffect(() => { if (!narrow) setTreeOpen(false); }, [narrow]);
  const focusTree = () => {
    // After the tree is drawn: its rows are laid out only once it shows.
    requestAnimationFrame(() => {
      const root = treeRef.current;
      const row = root?.querySelector<HTMLElement>('[role="treeitem"][aria-selected="true"]')
        ?? root?.querySelector<HTMLElement>('[role="treeitem"][tabindex="0"]')
        ?? root?.querySelector<HTMLElement>('[role="treeitem"]');
      row?.scrollIntoView?.({ block: "nearest" });
      row?.focus();
    });
  };
  const closeTree = (refocus: boolean) => {
    setTreeOpen(false);
    if (refocus) requestAnimationFrame(() => document.querySelector<HTMLElement>("[data-shell-crumb]")?.focus());
  };
  const goToTree = useEventCallback(() => {
    if (narrow) setTreeOpen(true);
    focusTree();
    return true;
  });
  useCommands({ goToTree });
  const onTreeKey = useEventCallback((event: KeyboardEvent) => {
    if (!treeKeyAction(event, platform, event.target as HTMLElement | null)) return;
    event.preventDefault();
    event.stopPropagation();
    if (!event.repeat) runCommand("goToTree");
  });
  useEffect(() => {
    window.addEventListener("keydown", onTreeKey, true);
    return () => window.removeEventListener("keydown", onTreeKey, true);
  }, [onTreeKey]);
  // A click outside the drawn-over tree puts it away (the breadcrumb toggles it itself).
  useEffect(() => {
    if (!treeOpen) return undefined;
    const away = (event: PointerEvent) => {
      const at = event.target as Element | null;
      if (treeRef.current?.contains(at) || at?.closest?.("[data-shell-crumb], [role=\"menu\"], [role=\"dialog\"]")) return;
      setTreeOpen(false);
    };
    document.addEventListener("pointerdown", away, true);
    return () => document.removeEventListener("pointerdown", away, true);
  }, [treeOpen]);
  const treeProps = treeOpen ? {
    // Escape puts it away; a row chosen (clicked, or Enter or Space on it) opens and puts it away.
    onKeyDownCapture: (event: ReactKeyboardEvent) => {
      const at = event.target as HTMLElement;
      if (isTyping(at)) return;
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeTree(true); return; }
      if ((event.key === "Enter" || event.key === " ") && at.getAttribute("role") === "treeitem" && !event.repeat) {
        setTimeout(() => closeTree(true), 0);
      }
    },
    onClick: (event: ReactMouseEvent) => {
      const at = event.target as HTMLElement;
      if (isTyping(at) || !at.closest('[role="treeitem"]')) return;
      closeTree(true);
    },
  } : {};

  return (
    <div className={styles.shell} data-testid="shell" data-view={view} ref={observeShellWidth}>
      <div className={styles.frame} data-tree-open={(treeOpen && narrow) || undefined}>
        <header className={styles.topBar} data-testid="shell-top-bar">
          <div className={styles.tabs} role="tablist" aria-label={t("Views")}>
            {SHELL_VIEWS.map((entry) => {
              const chord = shellViewChord(entry.id, keyOverrides);
              const key = chord.key === "" ? null : describeChord(chord, platform);
              return (
                <button
                  key={entry.id}
                  type="button"
                  role="tab"
                  id={tabId(entry.id)}
                  className={styles.tab}
                  aria-selected={entry.id === view}
                  aria-controls="shell-working"
                  aria-keyshortcuts={key ?? undefined}
                  // Arrow keys are the list's and the deck's, so the tabs are
                  // not a roving group: each is reached on its own.
                  onClick={() => { runCommand(entry.action); }}
                >
                  {key !== null ? <span className={styles.key} aria-hidden>{key}</span> : null}
                  <span>{t(entry.label)}</span>
                  {tabCounts?.[entry.id] != null ? (
                    <span className={styles.count} data-testid={`shell-tab-count-${entry.id}`}>{tabCounts[entry.id]}</span>
                  ) : null}
                </button>
              );
            })}
          </div>
          <div className={styles.topBarEnd} data-testid="shell-top-bar-end">{topBarEnd}</div>
        </header>
        <section className={styles.player} data-testid="shell-player" aria-label={t("Player")}>{player}</section>
        <aside className={styles.tree} data-testid="shell-tree" aria-label={t("Tree")} ref={treeRef} {...treeProps}>{tree}</aside>
        <main
          className={styles.working}
          id="shell-working"
          role="tabpanel"
          aria-labelledby={tabId(view)}
          data-testid="shell-working"
        >
          {view === "library" ? <LibraryView list={library} target={libraryTarget} crumb={libraryCrumb} /> : null}
          {view === "intake" ? intake?.(intakeQueue ?? "intake") ?? null : null}
          {view === "review" ? <ReviewView /> : null}
          {view === "stick" ? <StickView stick={stick} /> : null}
        </main>
        <footer className={styles.status} data-testid="shell-status">{status}</footer>
      </div>
    </div>
  );
}
