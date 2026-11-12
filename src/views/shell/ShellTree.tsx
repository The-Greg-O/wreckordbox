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
 * that opens and what the view shows. A Stick's libraries are read as soon
 * as the Stick appears (`onReadStick`), so its playlists are listed without
 * the DJ opening anything.
 *
 * Keys, while a row has the focus: Up and Down move between rows, Enter or
 * Space opens one, Left closes a folder and Right opens it.
 */
import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import type { TreeNode } from "@/ipc/types";
import { isShellTreePlace, shellTreeRows, type ShellSetPlan, type ShellTreeRow } from "@/lib/shellTree";
import styles from "./ShellTree.module.css";

export interface ShellTreeProps {
  /** The Library's tree followed by the mounted Sticks' nodes, as the window holds them. */
  nodes: readonly TreeNode[];
  /** Set plans, once they exist (ticket #76). */
  setPlans?: readonly ShellSetPlan[];
  /** The row shown as selected. */
  selectedId: string | null;
  /** A row was chosen. */
  onOpen: (row: ShellTreeRow) => void;
  /** A Stick is in the tree whose libraries may not have been read yet. */
  onReadStick: (node: TreeNode) => void;
  /** The paths of the Sticks whose libraries have been read. */
  readSticks?: ReadonlySet<string> | undefined;
}

/** Nothing produces Set plans yet; one empty list, so the rows are not remade each render. */
const NO_SET_PLANS: readonly ShellSetPlan[] = [];
const NONE_READ: ReadonlySet<string> = new Set();

interface Group {
  id: string;
  /** What the group is called, for a screen reader. */
  label: string;
  /** The heading line, when the group has one that is not a row. */
  heading?: string;
  rows: ShellTreeRow[];
}

/** The flat rows, cut into their groups: a heading, or a Stick, starts one. */
function groupRows(rows: readonly ShellTreeRow[], t: (text: string) => string): Group[] {
  const groups: Group[] = [];
  for (const row of rows) {
    if (row.kind === "heading") {
      groups.push({ id: row.id, label: t(row.name), heading: t(row.name), rows: [] });
    } else if (row.kind === "stick") {
      groups.push({ id: row.id, label: `${t("Stick")} ${row.name}`, rows: [row] });
    } else {
      groups.at(-1)?.rows.push(row);
    }
  }
  return groups;
}

/** The queues and the empty lines are the shell's own words; everything else is a name. */
function rowName(row: ShellTreeRow, t: (text: string) => string): string {
  return row.kind === "intake" || row.kind === "problems" || row.kind === "empty" ? t(row.name) : row.name;
}

export function ShellTree({ nodes, setPlans = NO_SET_PLANS, selectedId, onOpen, onReadStick, readSticks = NONE_READ }: ShellTreeProps) {
  const t = useTranslation();
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const treeRows = useMemo(
    () => shellTreeRows({ nodes, setPlans, collapsed, readSticks }),
    [nodes, setPlans, collapsed, readSticks],
  );
  const groups = useMemo(() => groupRows(treeRows, t), [treeRows, t]);
  const places = useMemo(() => treeRows.filter((row) => isShellTreePlace(row.kind)), [treeRows]);
  const root = useRef<HTMLDivElement>(null);

  // A Stick's playlists are listed as soon as it is mounted. Reading one
  // already read, or being read, does nothing.
  useEffect(() => {
    for (const node of nodes) if (node.kind === "device") onReadStick(node);
  }, [nodes, onReadStick]);

  const toggle = (id: string, open: boolean) => {
    setCollapsed((current) => {
      const next = new Set(current);
      if (open) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // One row takes the Tab key: the selected one, or the first.
  const tabStop = places.some((row) => row.id === selectedId) ? selectedId : places[0]?.id;

  const focusRow = (id: string | undefined) => {
    if (id === undefined) return;
    const element = root.current?.querySelector<HTMLElement>(`[data-row-id="${CSS.escape(id)}"]`);
    element?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLElement>, row: ShellTreeRow) => {
    const at = places.findIndex((place) => place.id === row.id);
    let handled = true;
    if (event.key === "ArrowDown") focusRow(places[at + 1]?.id);
    else if (event.key === "ArrowUp") focusRow(places[at - 1]?.id);
    else if (event.key === "Enter" || event.key === " ") onOpen(row);
    else if (event.key === "ArrowLeft" && row.branch && row.open) toggle(row.id, false);
    else if (event.key === "ArrowRight" && row.branch && !row.open) toggle(row.id, true);
    else handled = false;
    if (handled) {
      // The list and the deck listen on the window: these keys are the tree's.
      event.preventDefault();
      event.stopPropagation();
    }
  };

  return (
    <div className={styles.tree} role="tree" aria-label={t("Tree")} ref={root}>
      {groups.map((group) => (
        <div key={group.id} role="group" aria-label={group.label} className={styles.group}>
          {group.heading !== undefined ? <div className={styles.heading} aria-hidden>{group.heading}</div> : null}
          {group.rows.map((row) => {
            if (!isShellTreePlace(row.kind)) {
              return <div key={row.id} className={styles.empty}>{rowName(row, t)}</div>;
            }
            const name = rowName(row, t);
            const selected = row.id === selectedId;
            return (
              <div
                key={row.id}
                role="treeitem"
                aria-label={name}
                aria-level={row.depth + 1}
                aria-selected={selected}
                aria-expanded={row.branch ? row.open : undefined}
                data-kind={row.kind}
                data-row-id={row.id}
                tabIndex={row.id === tabStop ? 0 : -1}
                className={row.kind === "stick" ? styles.stick : styles.row}
                style={{ "--depth": row.depth } as CSSProperties}
                onClick={() => { onOpen(row); }}
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
                <span className={styles.name} data-name>{name}</span>
                {row.count !== undefined ? <span className={styles.count} aria-hidden>{row.count}</span> : null}
              </div>
            );
          })}
        </div>
      ))}
    </div>
  );
}
