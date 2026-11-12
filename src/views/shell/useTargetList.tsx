/**
 * The Library view's second list, as the window keeps it (wbx-59).
 *
 * The one call `AppBody` makes. It holds whether the list is shown (hidden
 * by default, every session), the target playlist, and the target's
 * selected row; it reads the target's rows; it answers the list's keys and
 * registers its commands for the command palette; and it returns the pane
 * for the Library view and the target's id for the tree to mark.
 *
 * Adding and reordering are the window's existing playlist edits, handed
 * in by `AppBody` (`add` is Add To Playlist, a job with the duplicates
 * question and one Undo step; `reorder` is Reorder Playlist). A locked
 * Library is refused here with the window's own words, before anything is
 * sent, and the backend refuses it again behind that.
 *
 * Nothing here writes to a Stick: a Stick's playlist cannot be the target
 * until Stick Protection exists (`targetRefusal`), and tracks listed from
 * a Stick are not Library tracks, so they are not added.
 */
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import type { TreeNode, ViewSpec } from "@/ipc/types";
import { SHELL_NARROW_MAX_PX } from "@/lib/shellViews";
import type { Platform } from "@/lib/shortcuts";
import type { KeyDisplay } from "@/ipc/types";
import { moveInOrder, swapLists, targetCandidates, targetKeyAction } from "@/lib/targetList";
import type { Action } from "@/lib/shortcuts";
import { useCommands } from "@/store/useCommands";
import { useEventCallback } from "@/store/useEventCallback";
import { TargetList } from "./TargetList";
import { useTargetRows } from "./useTargetRows";

export interface TargetListInput {
  /** The shell shows the Library view: the list's keys and commands are live only then. */
  active: boolean;
  platform: Platform;
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
  /** Reorder Playlist, given the whole new order. */
  reorder: (playlist: string, order: readonly string[]) => void;
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
  const { active, platform, tree, source, sourceKind, onOpen, selection, locked, explainLock, refuse, add, reorder, dragged, endDrag } = input;
  const [shown, setShown] = useState(false);
  const [chosen, setChosen] = useState<string | null>(null);
  const [selected, setSelected] = useState<number | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const narrow = useNarrow();

  const candidates = useMemo(() => targetCandidates(tree), [tree]);
  // A target deleted from the Library is no target.
  const targetId = chosen !== null && candidates.some((candidate) => candidate.id === chosen) ? chosen : null;
  const targetNode = targetId === null ? null : tree.find((node) => node.id === targetId) ?? null;
  const visible = active && shown && !narrow;
  const { rows, failed, retry } = useTargetRows(visible ? targetId : null);

  useEffect(() => {
    if (rows !== null && selected !== null && selected >= rows.length) setSelected(rows.length === 0 ? null : rows.length - 1);
  }, [rows, selected]);

  const refused = useCallback((message: string) => {
    setNotice(message);
    refuse(message);
    return true;
  }, [refuse]);

  const choose = useCallback((id: string | null) => {
    setChosen(id);
    setSelected(null);
    setNotice(null);
  }, []);

  const toggle = useEventCallback(() => {
    if (!shown && narrow) return refused("The window is too narrow for a second list: widen it past 800 pixels.");
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

  const applyOrder = useEventCallback((order: readonly string[], index: number) => {
    if (targetId === null) return refused("Choose a target playlist first.");
    if (failed !== null) return refused("The target could not be read, so it is not reordered: try again first.");
    if (locked) {
      explainLock();
      return true;
    }
    setSelected(index);
    setNotice(null);
    reorder(targetId, order);
    return true;
  });

  const move = useEventCallback((delta: -1 | 1) => {
    if (rows === null || targetId === null) return refused("Choose a target playlist first.");
    if (selected === null) return refused("Select a track in the target first.");
    const moved = moveInOrder(rows.map((row) => row.id), selected, delta);
    // At either end there is nowhere to go; the key is still the list's.
    if (moved === null) return true;
    return applyOrder(moved.order, moved.index);
  });

  const swap = useEventCallback(() => {
    const plan = swapLists(source, targetNode);
    if ("refusal" in plan) return refused(plan.refusal);
    setChosen(plan.target.id);
    setSelected(null);
    setNotice(null);
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
  const onKey = useEventCallback((event: KeyboardEvent) => {
    const action = targetKeyAction(event, platform, event.target as HTMLElement | null);
    if (action === null) return;
    if (action !== "toggleTargetList" && !visible) return;
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
      onReorder={(order, index) => { applyOrder(order, index); }}
      dragging={dragged !== null}
      onDropTracks={dropDragged}
      notice={notice}
      keyDisplay={input.keyDisplay}
      loadedKey={input.loadedKey}
      platform={platform}
    />
  ) : null;
  return { pane, targetId: visible ? targetId : null };
}
