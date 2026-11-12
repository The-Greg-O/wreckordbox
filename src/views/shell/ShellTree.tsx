/**
 * The view shell's tree (ticket #52): everything the DJ can open, in one
 * tree, in every view.
 *
 * Four kinds of group, top to bottom: the Library (All Tracks and the Intake
 * and Problems queues), Playlists (every folder, playlist and intelligent
 * playlist, at any depth), Set plans, and one group per mounted Stick headed
 * by the Stick itself. What the rows are is decided by `shellTreeRows`
 * (`src/lib/shellTree.ts`); this draws them and says which one was chosen.
 *
 * Selecting a row is the window's business (`onOpen`): it decides which view
 * that opens and what the view shows. The Sticks' playlists come in already
 * read (`useStickOutlines`); nothing here reads a Stick.
 *
 * Only the rows in view are drawn, so a Library of any size opens at once:
 * every row is one fixed height (the shell's row and heading tokens, read
 * from the stylesheet), which places each without measuring it.
 *
 * Keys, while a row has the focus (`keyTarget` in `shellTreeNav.ts`): Up and
 * Down move between rows, Home and End to the first and last, Left closes a
 * folder or goes to the parent, Right opens a folder or goes into it, Enter
 * or Space opens the row. Each is the tree's even where it has nowhere to
 * go, so the deck's jumps and the list's moves never hear it. Which row has
 * the focus is kept apart from which is selected: the keyboard moves the
 * one and only Enter changes the other.
 */
import { useFontScales } from "@/lib/themeRoles";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import type { TreeNode } from "@/ipc/types";
import { isShellTreePlace, shellTreeRows, type ShellSetPlan, type ShellStick, type ShellTreeRow } from "@/lib/shellTree";
import { keyTarget, treeLayout, visibleRange, type TreeSizes } from "@/lib/shellTreeNav";
import styles from "./ShellTree.module.css";
import { useShellTreeActions, type ShellTreeActionProps } from "./ShellTreeActions";
import { setBrowserFocus } from "@/lib/browserFocus";

export interface ShellTreeProps {
  /** The Library's tree, as the backend sends it. */
  library: readonly TreeNode[];
  /** The mounted Sticks, with their playlists once read. */
  sticks: readonly ShellStick[];
  /** Set plans, once they exist (ticket #76). */
  setPlans?: readonly ShellSetPlan[];
  /** The row shown as selected. */
  selectedId: string | null;
  /** A row was chosen. */
  onOpen: (row: ShellTreeRow) => void;
  /** The old tree's actions (wbx-45.4): menus, rename, drops, moves, Eject. None without. */
  actions?: ShellTreeActionProps | undefined;
  /** The Library node selected, which the Playlist menu's commands act on. */
  selectedNode?: TreeNode | null | undefined;
  /** The folders closed, when the window keeps them (the session); kept here otherwise. */
  collapsed?: ReadonlySet<string> | undefined;
  /** A folder was opened or closed. */
  onFolderOpen?: ((id: string, open: boolean) => void) | undefined;
  /** Why a Stick may not be written to now (the Stick write gate), or null. */
  stickRefusal?: ((path: string) => string | null) | undefined;
  /** What waits in Intake and Problems, drawn beside them (wbx-62). */
  queueCounts?: { intake: number | null; problems: number | null } | null | undefined;
  /** The second list's target playlist, marked in orange (wbx-59). */
  targetId?: string | null;
}

/** No actions: a tree that only opens rows. */
const NO_ACTIONS: ShellTreeActionProps = {};

/** Nothing produces Set plans yet; one empty list, so the rows are not remade each render. */
const NO_SET_PLANS: readonly ShellSetPlan[] = [];

/**
 * Levels of indent drawn: deeper rows sit at this one, so a name deep in
 * folders keeps the width to be read. The real depth is still `aria-level`.
 */
const MAX_INDENT = 5;
/** Rows drawn beyond each edge of the view, so a scroll does not show a gap. */
const SPARE_ROWS = 10;
/** The sizes at 100%, should the stylesheet not say (a test without it). */
const FALLBACK_SIZES: TreeSizes = { row: 24, heading: 22, gap: 10, pad: 8 };

interface Group {
  id: string;
  /** What the group is called, for a screen reader. */
  label: string;
  /** The heading line, when the group has one that is not a row. */
  heading?: string;
  /** The heading's own row, for a heading the keyboard can reach. */
  headingRow?: ShellTreeRow;
  rows: ShellTreeRow[];
}

/** The flat rows, cut into their groups: a heading, or a Stick, starts one. */
function groupRows(rows: readonly ShellTreeRow[], t: (text: string) => string): Group[] {
  const groups: Group[] = [];
  for (const row of rows) {
    if (row.kind === "heading") {
      groups.push({ id: row.id, label: t(row.name), heading: t(row.name), headingRow: row, rows: [] });
    } else if (row.kind === "stick") {
      groups.push({ id: row.id, label: `${t("Stick")} ${row.name}`, rows: [row] });
    } else {
      groups.at(-1)?.rows.push(row);
    }
  }
  return groups;
}

/** The queues, the libraries' names and the empty lines are the shell's own words; everything else is a name. */
function rowName(row: ShellTreeRow, t: (text: string) => string): string {
  return row.kind === "intake" || row.kind === "problems" || row.kind === "empty" || row.kind === "stickLibrary"
    ? t(row.name)
    : row.name;
}

/** The tree's sizes, from the shell's tokens. */
function readSizes(element: HTMLElement | null): TreeSizes {
  if (element === null) return FALLBACK_SIZES;
  const style = getComputedStyle(element);
  const px = (name: string, fallback: number) => {
    const value = Number.parseFloat(style.getPropertyValue(name));
    return Number.isFinite(value) && value > 0 ? value : fallback;
  };
  return {
    row: px("--s-shell-row-h", FALLBACK_SIZES.row),
    heading: px("--s-shell-heading-h", FALLBACK_SIZES.heading),
    gap: px("--s-shell-space-5", FALLBACK_SIZES.gap),
    pad: px("--s-shell-space-4", FALLBACK_SIZES.pad),
  };
}

export function ShellTree({
  library, sticks, setPlans = NO_SET_PLANS, selectedId, onOpen, queueCounts, actions = NO_ACTIONS, selectedNode = null,
  collapsed: keptCollapsed, onFolderOpen, stickRefusal, targetId = null,
}: ShellTreeProps) {
  const t = useTranslation();
  const [ownCollapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const collapsed = keptCollapsed ?? ownCollapsed;
  const act = useShellTreeActions({ actions, library, sticks, selectedNode, stickRefusal });
  const treeRows = useMemo(
    () => shellTreeRows({ library, sticks, setPlans, collapsed, queueCounts }),
    [library, sticks, setPlans, collapsed, queueCounts],
  );
  const groups = useMemo(() => groupRows(treeRows, t), [treeRows, t]);
  const root = useRef<HTMLDivElement>(null);

  const [sizes, setSizes] = useState<TreeSizes>(FALLBACK_SIZES);
  const [view, setView] = useState({ scroll: 0, height: 0 });
  useLayoutEffect(() => {
    const element = root.current;
    if (element === null) return undefined;
    setSizes(readSizes(element));
    const measure = () => { setView({ scroll: element.scrollTop, height: element.clientHeight }); };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => { observer.disconnect(); };
  }, []);
  // The theme file's text size changes the row heights (wbx-45.6).
  const fontScales = useFontScales();
  useLayoutEffect(() => {
    if (root.current !== null) setSizes(readSizes(root.current));
  }, [fontScales]);
  const layout = useMemo(
    () => treeLayout(groups.map((group) => [group.heading !== undefined, group.rows.length] as const), sizes),
    [groups, sizes],
  );

  // The row with the keyboard's focus, apart from the selected one.
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const isPlace = useCallback((id: string | null) => id !== null && treeRows.some((row) => row.id === id && isShellTreePlace(row.kind)), [treeRows]);
  // Where the keyboard may land: the places, and what opens a menu without
  // being one (the Playlists heading, a Stick library's label; wbx-45.4).
  const isStop = (row: ShellTreeRow) => isShellTreePlace(row.kind) || act.stop(row);
  const tabStop = isPlace(focusedId) ? focusedId : isPlace(selectedId) ? selectedId : treeRows.find((row) => isShellTreePlace(row.kind))?.id ?? null;

  // A row the keyboard moved to may not be drawn yet: scroll it into view,
  // then focus it once it is.
  const wantFocus = useRef<string | null>(null);
  const moveFocus = (id: string) => {
    setFocusedId(id);
    wantFocus.current = id;
    const element = root.current;
    if (element === null) return;
    groups.forEach((group, g) => {
      const index = group.rows.findIndex((row) => row.id === id);
      const box = layout.groups[g];
      if (index < 0 || box === undefined) return;
      const top = box.rowsTop + index * sizes.row;
      if (top < element.scrollTop) element.scrollTop = top - (group.heading !== undefined && index === 0 ? sizes.heading : 0);
      else if (top + sizes.row > element.scrollTop + element.clientHeight) element.scrollTop = top + sizes.row - element.clientHeight;
      setView({ scroll: element.scrollTop, height: element.clientHeight });
    });
  };
  useEffect(() => {
    const id = wantFocus.current;
    if (id === null) return;
    const element = root.current?.querySelector<HTMLElement>(`[data-row-id="${CSS.escape(id)}"]`);
    if (element) {
      wantFocus.current = null;
      element.focus({ preventScroll: true });
    }
  });

  const toggle = (id: string, open: boolean) => {
    onFolderOpen?.(id, open);
    if (keptCollapsed !== undefined) return;
    setCollapsed((current) => {
      const next = new Set(current);
      if (open) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>, row: ShellTreeRow) => {
    // Keys typed into the rename field are the field's.
    if (event.target !== event.currentTarget) return;
    if (act.key(event, row)) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === "Enter" || event.key === " ") {
      onOpen(row);
    } else {
      const target = keyTarget(treeRows, row.id, event.key, isStop);
      if (target === null) return;
      if ("focus" in target) moveFocus(target.focus);
      else if ("toggle" in target) toggle(target.toggle, target.open);
    }
    // The list and the deck listen on the window: these keys are the tree's.
    event.preventDefault();
    event.stopPropagation();
  };

  const drawRow = (row: ShellTreeRow) => {
    const name = rowName(row, t);
    if (!isShellTreePlace(row.kind) && act.stop(row)) {
      // Not a place, but its menu is the way to make a playlist in that library.
      return (
        <div
          key={row.id}
          role="treeitem"
          aria-label={name}
          aria-level={1}
          data-kind={row.kind}
          data-row-id={row.id}
          tabIndex={-1}
          className={styles.library}
          {...act.row(row)}
          onFocus={() => { setFocusedId(row.id); }}
          onKeyDown={(event) => { onKeyDown(event, row); }}
        >
          {name}
        </div>
      );
    }
    if (!isShellTreePlace(row.kind)) {
      return <div key={row.id} className={row.kind === "stickLibrary" ? styles.library : styles.empty}>{name}</div>;
    }
    const selected = row.id === selectedId;
    return (
      <div
        key={row.id}
        role="treeitem"
        aria-label={name}
        aria-level={row.depth + 1}
        aria-selected={selected}
        aria-expanded={row.branch ? row.open : undefined}
        aria-description={row.id === targetId ? t("Target playlist") : undefined}
        data-target={row.id === targetId || undefined}
        data-kind={row.kind}
        data-row-id={row.id}
        tabIndex={row.id === tabStop ? 0 : -1}
        title={name}
        className={row.kind === "stick" ? styles.stick : styles.row}
        style={{ "--depth": Math.min(row.depth, MAX_INDENT) } as CSSProperties}
        {...act.row(row)}
        onClick={() => { onOpen(row); }}
        onFocus={() => { setFocusedId(row.id); }}
        onKeyDown={(event) => { onKeyDown(event, row); }}
      >
        {row.branch ? (
          <button
            type="button"
            tabIndex={-1}
            className={styles.twisty}
            aria-label={`${row.open ? t("Collapse") : t("Expand")} ${name}`}
            onClick={(event) => {
              event.stopPropagation();
              toggle(row.id, !row.open);
            }}
          >
            {row.open ? "▾" : "▸"}
          </button>
        ) : null}
        {row.kind === "stick" ? <span className={styles.stickLabel} aria-hidden>{t("Stick")} ·</span> : null}
        {act.rename(row, name) ?? <span className={styles.name} data-name>{name}</span>}
        {row.count !== undefined ? <span className={styles.count} aria-hidden>{row.count}</span> : null}
        {row.protection === undefined ? null : (
          <span className={styles.protection} data-stick-protection={row.protection}
            title={row.protection === "protected" ? t("Protected: nothing is written to it until it is unlocked") : t("Unlocked until it is ejected or removed")}>
            {row.protection === "protected" ? t("Protected") : t("Unlocked")}
          </span>
        )}
        {act.end(row)}
      </div>
    );
  };

  return (
    <div
      className={styles.tree}
      role="tree"
      aria-label={t("Tree")}
      ref={root}
      // The window's browser focus (the palette and the menus answer the
      // tree it is in): a click is followed by the window; a key-only
      // arrival is told here.
      data-browser-pane="tree"
      data-browser-side="main"
      onFocus={() => { setBrowserFocus({ pane: "tree", side: "main" }); }}
      onScroll={(event) => { setView({ scroll: event.currentTarget.scrollTop, height: event.currentTarget.clientHeight }); }}
    >
      <div className={styles.content} style={{ height: layout.height }}>
        {groups.map((group, g) => {
          const box = layout.groups[g];
          if (box === undefined) return null;
          // Before the first measure, draw a screenful.
          const height = view.height > 0 ? view.height : 1000;
          const [first, end] = visibleRange(box.rowsTop, group.rows.length, sizes.row, view.scroll, height, SPARE_ROWS);
          const inView = box.top + box.height >= view.scroll - SPARE_ROWS * sizes.row && box.top <= view.scroll + height + SPARE_ROWS * sizes.row;
          if (!inView) return null;
          return (
            <div
              key={group.id}
              role="group"
              aria-label={group.label}
              className={styles.group}
              style={{ top: box.top, height: box.height }}
            >
              {group.heading !== undefined && group.headingRow !== undefined && act.stop(group.headingRow) ? (
                <div
                  className={styles.heading}
                  role="treeitem"
                  aria-label={group.heading}
                  aria-level={1}
                  aria-haspopup="menu"
                  data-kind="collection"
                  data-row-id={group.headingRow.id}
                  tabIndex={-1}
                  {...act.heading(group.id)}
                  onFocus={() => { if (group.headingRow) setFocusedId(group.headingRow.id); }}
                  onKeyDown={(event) => { if (group.headingRow) onKeyDown(event, group.headingRow); }}
                >
                  {group.heading}
                </div>
              ) : group.heading !== undefined ? <div className={styles.heading} aria-hidden>{group.heading}</div> : null}
              <div style={{ height: first * sizes.row }} aria-hidden />
              {group.rows.slice(first, end).map(drawRow)}
            </div>
          );
        })}
      </div>
      {act.menu}
    </div>
  );
}
