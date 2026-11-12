/**
 * The shell's tree wired to the window (ticket #52), so the window adds one
 * call rather than the tree's bookkeeping.
 *
 * Choosing a row opens it in its view (`shellViewForRow`): a Library node
 * becomes the window's selected node, as the old tree's selection did, and
 * Intake or Problems becomes the queue the Intake view shows. The queue is
 * remembered, so the Intake tab and its key come back to the one last
 * chosen.
 *
 * The tree shows one row selected: the queue, while the Intake view shows it
 * and it was the last row chosen in the tree; otherwise the selected node.
 * So a view switch by tab or key leaves the tree's selection where it was
 * (the selected node belongs to the shell and survives a switch), and
 * choosing a queue in the tree shows that queue as the selection.
 */
import { useCallback, useMemo, useState, type ReactNode } from "react";

import type { TreeNode } from "@/ipc/types";
import { shellTreeQueueId, shellViewForRow, type IntakeQueue, type ShellStick, type ShellTreeRow } from "@/lib/shellTree";
import type { TreeExpansion } from "@/lib/session";
import { deleteQuestion, setShellFolderOpen, shellCollapsed } from "@/lib/shellTreeActions";
import type { ShellView } from "@/lib/shellViews";
import { ShellTree } from "./ShellTree";
import type { ShellTreeActionProps } from "./ShellTreeActions";
import type { StickWriteGate } from "./StickWriteGate";

export function useShellTree({
  library, sticks, selectedNode, onSelectNode, view, onViewChange, queueCounts, actions, fromStick = false, expansion, onExpansionChange,
  stickGate, confirmDelete, targetId,
}: {
  /** The Library's tree, as the backend sends it. */
  library: readonly TreeNode[];
  /** The mounted Sticks, as `useStickOutlines` reads them. */
  sticks: readonly ShellStick[];
  selectedNode: TreeNode | null;
  onSelectNode: (node: TreeNode) => void;
  view: ShellView;
  onViewChange: (view: ShellView) => void;
  /** The old tree's handlers (wbx-45.4). */
  actions?: ShellTreeActionProps;
  /** Tracks in hand may go on a Stick's playlist, a Stick's own included. */
  fromStick?: boolean;
  /** The folders' open and closed state, kept in the session as the old tree's is. */
  expansion?: TreeExpansion;
  onExpansionChange?: (expansion: TreeExpansion) => void;
  /**
   * The window's Stick write gate (wbx-45.4): the actions' Stick writes are
   * staged by the window's own handlers; the tree asks it which Sticks may
   * take a drop or an export. Without it the tree offers no Stick writes.
   */
  stickGate?: StickWriteGate;
  /** Asks before a Library playlist or folder is deleted (wbx-45.4 review, finding 8). Without it nothing is deleted from the tree. */
  confirmDelete?: (question: string) => Promise<boolean>;
  /** What waits in Intake and Problems, drawn beside them in the tree (wbx-62). */
  queueCounts?: { intake: number | null; problems: number | null } | null | undefined;
  /** The second list's target playlist, marked in the tree (wbx-59). */
  targetId?: string | null | undefined;
}): { tree: ReactNode; intakeQueue: IntakeQueue; showQueue: (queue: IntakeQueue) => void } {
  const [intakeQueue, setIntakeQueue] = useState<IntakeQueue>("intake");
  const [queueChosen, setQueueChosen] = useState(false);

  const open = useCallback((row: ShellTreeRow) => {
    setQueueChosen(row.queue !== undefined);
    if (row.queue !== undefined) setIntakeQueue(row.queue);
    else if (row.node !== undefined) onSelectNode(row.node);
    onViewChange(shellViewForRow(row.kind));
  }, [onSelectNode, onViewChange]);

  /** Shows a queue in the Intake view as choosing it in the tree does (the empty queue's offer of Problems). */
  const showQueue = useCallback((queue: IntakeQueue) => {
    setQueueChosen(true);
    setIntakeQueue(queue);
    onViewChange("intake");
  }, [onViewChange]);

  const selectedId = view === "intake" && queueChosen ? shellTreeQueueId(intakeQueue) : selectedNode?.id ?? null;
  const treeActions = useMemo(() => {
    if (!actions) return undefined;
    const { onDeleteNode: remove, ...kept } = actions;
    const base: ShellTreeActionProps = {
      ...kept,
      canDropToDevicePlaylists: fromStick,
      ...(remove && confirmDelete
        ? { onDeleteNode: (node: TreeNode) => {
          void confirmDelete(deleteQuestion(library, node)).then((yes) => { if (yes) remove(node); });
        } }
        : {}),
    };
    if (stickGate) return base;
    // No gate: nothing in the tree may write to a Stick.
    const { onExport: _e, onDropTracksOnDevice: _t, onDropDeviceTracks: _d, onDeviceCreate: _c, onDeviceRename: _r, onDeviceDelete: _x, onDeviceImport: _i, ...rest } = base;
    return rest;
  }, [actions, fromStick, stickGate, confirmDelete, library]);
  const collapsed = useMemo(() => (expansion ? shellCollapsed(expansion) : undefined), [expansion]);
  const folderOpen = useCallback((id: string, isOpen: boolean) => {
    if (expansion && onExpansionChange) onExpansionChange(setShellFolderOpen(expansion, id, isOpen));
  }, [expansion, onExpansionChange]);
  const tree = (
    <ShellTree
      library={library} sticks={sticks} selectedId={selectedId} onOpen={open}
      actions={treeActions} selectedNode={selectedNode} collapsed={collapsed} onFolderOpen={folderOpen}
      stickRefusal={stickGate?.refusal} queueCounts={queueCounts} targetId={targetId ?? null}
    />
  );
  return { tree, intakeQueue, showQueue };
}
