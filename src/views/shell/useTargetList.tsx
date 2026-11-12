/**
 * The Library view's second list, as the window keeps it (wbx-59).
 *
 * The one call `AppBody` makes. It holds whether the list is shown (hidden
 * by default, every session), the target playlist, and the target's
 * selected entry; it reads the target's entries; it answers the list's keys
 * and registers its commands for the command palette; and it returns the
 * pane for the Library view and the target's id for the tree to mark.
 *
 * Adding is the window's Add To Playlist (`add`: a job with the duplicates
 * question and one Undo step). Reordering is by entry
 * (`reorderEntries`, the Library's `reorder_playlist_entries`), so a track
 * that is in the playlist twice is two rows that move on their own and keep
 * their identity. A locked Library is refused here with the window's own
 * words, before anything is sent, and the backend refuses it again behind
 * that.
 *
 * Reorders are shown at once and written one at a time, in the order they
 * were asked for: each write sends the whole order as shown when it was
 * asked, so two presses before the Library answers make the order both
 * presses describe. The shown order is kept until a read made after the
 * last write arrives. A write that fails drops every write still waiting,
 * puts back the order the Library has, and keeps the selection on the same
 * entry.
 *
 * Nothing here writes to a Stick: a Stick's playlist cannot be the target
 * until Stick Protection exists (`targetRefusal`), and tracks listed from
 * a Stick are not Library tracks, so they are not added.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import type { KeyDisplay, TreeNode, ViewSpec } from "@/ipc/types";
import { browserFocus } from "@/lib/browserFocus";
import { SHELL_NARROW_MAX_PX } from "@/lib/shellViews";
import type { Action, KeyOverrides, Platform } from "@/lib/shortcuts";
import {
  moveInOrder, sourceOwnsKeyboard, swapLists, targetCandidates, targetKeyAction,
} from "@/lib/targetList";
import { useCommands } from "@/store/useCommands";
import { useEventCallback } from "@/store/useEventCallback";
import { TargetList } from "./TargetList";
import { useTargetRows, type TargetRow } from "./useTargetRows";

export interface TargetListInput {
  /** The shell shows the Library view: the list's keys and commands are live only then. */
  active: boolean;
  platform: Platform;
  /** The DJ's own keys, which the second list's navigation follows. */
  keyOverrides?: KeyOverrides | undefined;
  /** The Library's tree. */
  tree: readonly TreeNode[];
  /** What the main list shows. */
  source: TreeNode | null;
  sourceKind: ViewSpec["source"]["kind"];
  /** Opens a node in the main list, as choosing it in the tree does. */
  onOpen: (node: TreeNode) => void;
  /** The main list's selected tracks, hidden ones already dropped by the Track Filter. */
  selection: readonly { id: string }[];
  /** The Library cannot be edited now (Library Protection, or rekordbox holds it). */
  locked: boolean;
  /** Says why the Library cannot be edited, as every refused edit does. */
  explainLock: () => void;
  /** Says why something was refused, in the status line. */
  refuse: (message: string) => void;
  /** Add To Playlist. */
  add: (playlist: string, ids: readonly string[]) => void;
  /** Reorder by entry: resolves whether it was written (a refusal is reported by it). */
  reorderEntries: (playlist: string, entries: readonly string[]) => Promise<boolean>;
  /** Tracks being dragged from the main list, if any. */
  dragged: { ids: readonly string[]; sourceKind: ViewSpec["source"]["kind"] } | null;
  /** The drag has been used up (or refused): forget it. */
  endDrag: () => void;
  keyDisplay: KeyDisplay;
  loadedKey: string | null;
}

const NARROW = `(max-width: ${SHELL_NARROW_MAX_PX}px)`;

/** Whether the window is too narrow for the second list (`docs/design/tokens.md`, Breakpoints). */
function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => typeof matchMedia === "function" && matchMedia(NARROW).matches);
  useEffect(() => {
    if (typeof matchMedia !== "function") return undefined;
    const query = matchMedia(NARROW);
    const follow = () => { setNarrow(query.matches); };
    follow();
    query.addEventListener("change", follow);
    return () => query.removeEventListener("change", follow);
  }, []);
  return narrow;
}

/** Tracks a list shows from a Stick's own database: their ids are not the Library's. */
const fromStick = (kind: ViewSpec["source"]["kind"]) => kind === "device";

export function useTargetList(input: TargetListInput): { pane: ReactNode; targetId: string | null } {
  const {
    active, platform, keyOverrides, tree, source, sourceKind, onOpen, selection, locked, explainLock, refuse, add,
    reorderEntries, dragged, endDrag,
  } = input;
  const [shown, setShown] = useState(false);
  const [chosen, setChosen] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // The order shown while reorders are being written, by entry.
  const [pending, setPending] = useState<readonly string[] | null>(null);
  const narrow = useNarrow();

  const candidates = useMemo(() => targetCandidates(tree), [tree]);
  // A target deleted from the Library is no target, and the list says so.
  const targetId = chosen !== null && candidates.some((candidate) => candidate.id === chosen) ? chosen : null;
  const lost = chosen !== null && targetId === null && tree.length > 0;
  const targetNode = targetId === null ? null : tree.find((node) => node.id === targetId) ?? null;
  const visible = active && shown && !narrow;
  const { rows: confirmed, failed, readNo, readsStarted, retry } = useTargetRows(visible ? targetId : null);

  // The writes: one at a time, in order. `epoch` changes when the target
  // changes or a write fails, so writes still waiting are dropped.
  const queue = useRef<Promise<void>>(Promise.resolve());
  const waiting = useRef(0);
  const epoch = useRef(0);
  // A read started after this number shows every write made.
  const [settledAfter, setSettledAfter] = useState<number | null>(null);

  const rows: readonly TargetRow[] | null = useMemo(() => {
    if (confirmed === null || pending === null) return confirmed;
    const byEntry = new Map(confirmed.map((row) => [row.entryId, row]));
    const shownRows = pending.map((entry) => byEntry.get(entry)).filter((row): row is TargetRow => row !== undefined);
    return shownRows.length === confirmed.length ? shownRows : confirmed;
  }, [confirmed, pending]);

  // The shown order gives way to the Library's once a read made after the
  // last write has arrived.
  useEffect(() => {
    if (pending !== null && waiting.current === 0 && settledAfter !== null && readNo > settledAfter) {
      setPending(null);
      setSettledAfter(null);
    }
  }, [pending, readNo, settledAfter]);

  // The selection is an entry: it is dropped when the entry is gone.
  useEffect(() => {
    if (selected !== null && confirmed !== null && !confirmed.some((row) => row.entryId === selected)) setSelected(null);
  }, [confirmed, selected]);

  const refused = useCallback((message: string) => {
    setNotice(message);
    refuse(message);
    return true;
  }, [refuse]);

  const forgetWrites = useCallback(() => {
    epoch.current += 1;
    setPending(null);
    setSettledAfter(null);
  }, []);

  const choose = useCallback((id: string | null) => {
    forgetWrites();
    setChosen(id);
    setSelected(null);
    setNotice(null);
  }, [forgetWrites]);

  const toggle = useEventCallback(() => {
    if (narrow) return refused("The window is too narrow for a second list: widen it past 800 pixels.");
    setShown(!shown);
    setNotice(null);
    return true;
  });

  const addTo = useEventCallback((ids: readonly string[], kind: ViewSpec["source"]["kind"]) => {
    if (targetId === null) return refused("Choose a target playlist first.");
    if (ids.length === 0) return refused("Select tracks in the list first.");
    if (fromStick(kind)) return refused("Tracks listed from a Stick cannot be added to a Library playlist.");
    if (locked) {
      explainLock();
      return true;
    }
    setNotice(null);
    add(targetId, ids);
    return true;
  });

  const addSelected = useEventCallback(() => addTo(selection.map((track) => track.id), sourceKind));

  const dropDragged = useEventCallback(() => {
    const drag = dragged;
    endDrag();
    if (drag) addTo(drag.ids, drag.sourceKind);
  });

  const applyOrder = useEventCallback((order: readonly string[], entry: string) => {
    if (targetId === null) return refused("Choose a target playlist first.");
    if (failed !== null) return refused("The target could not be read, so it is not reordered: try again first.");
    if (locked) {
      explainLock();
      return true;
    }
    setPending(order);
    setSelected(entry);
    setNotice(null);
    const playlist = targetId;
    const mine = epoch.current;
    waiting.current += 1;
    queue.current = queue.current.then(async () => {
      try {
        if (epoch.current !== mine) return;
        const written = await reorderEntries(playlist, order);
        if (!written && epoch.current === mine) {
          // Every write still waiting was made on top of this one.
          forgetWrites();
          retry();
        }
      } finally {
        waiting.current -= 1;
        if (waiting.current === 0 && epoch.current === mine) {
          // A read already under way may have begun before the last write
          // landed; ask for one that certainly begins after it.
          setSettledAfter(readsStarted());
          retry();
        }
      }
    });
    return true;
  });

  const move = useEventCallback((delta: -1 | 1) => {
    if (rows === null || targetId === null) return refused("Choose a target playlist first.");
    const at = selected === null ? -1 : rows.findIndex((row) => row.entryId === selected);
    if (at < 0) return refused("Select a track in the target first.");
    const moved = moveInOrder(rows.map((row) => row.entryId), at, delta);
    // At either end there is nowhere to go; the key is still the list's.
    if (moved === null) return true;
    return applyOrder(moved.order, selected!);
  });

  const swap = useEventCallback(() => {
    const plan = swapLists(source, targetNode);
    if ("refusal" in plan) return refused(plan.refusal);
    choose(plan.target.id);
    onOpen(plan.open);
    return true;
  });

  const run = useEventCallback((action: Action): boolean => {
    if (action === "toggleTargetList") return toggle();
    if (!visible) return false;
    switch (action) {
      case "addToTarget": return addSelected();
      case "moveTargetUp": return move(-1);
      case "moveTargetDown": return move(1);
      case "swapTargetLists": return swap();
      default: return false;
    }
  });

  useCommands({
    toggleTargetList: () => run("toggleTargetList"),
    addToTarget: () => run("addToTarget"),
    moveTargetUp: () => run("moveTargetUp"),
    moveTargetDown: () => run("moveTargetDown"),
    swapTargetLists: () => run("swapTargetLists"),
  }, undefined, active);

  // The keys, heard on the way down (capture) so Enter adds rather than
  // loading the deck, and Alt + Up / Down reorders the target rather than
  // moving the main list's cursor. Only while the Library view shows, and
  // (but for the key that shows the list) only while the list is shown.
  // Enter is an add only while the main list holds the keyboard: in the
  // tree it opens the focused playlist, in the second list it is the list's.
  const onKey = useEventCallback((event: KeyboardEvent) => {
    const focus = event.target as HTMLElement | null;
    const action = targetKeyAction(event, platform, focus);
    if (action === null) return;
    if (action !== "toggleTargetList" && !visible) return;
    if (action === "addToTarget" && !sourceOwnsKeyboard(focus, browserFocus())) return;
    const once = action === "toggleTargetList" || action === "swapTargetLists" || action === "addToTarget";
    event.preventDefault();
    event.stopPropagation();
    if (once && event.repeat) return;
    run(action);
  });
  useEffect(() => {
    if (!active) return undefined;
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [active, onKey]);

  const pane = visible ? (
    <TargetList
      candidates={candidates}
      targetId={targetId}
      onChoose={choose}
      rows={rows}
      failed={failed}
      onRetry={retry}
      selected={selected}
      onSelect={setSelected}
      onReorder={(order, entry) => { applyOrder(order, entry); }}
      dragging={dragged !== null}
      onDropTracks={dropDragged}
      notice={notice ?? (lost ? "The target playlist is no longer in the Library: choose another." : null)}
      keyDisplay={input.keyDisplay}
      loadedKey={input.loadedKey}
      platform={platform}
      keyOverrides={keyOverrides}
    />
  ) : null;
  return { pane, targetId: visible ? targetId : null };
}
