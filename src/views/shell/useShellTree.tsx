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
import { setShellFolderOpen, shellCollapsed } from "@/lib/shellTreeActions";
import type { ShellView } from "@/lib/shellViews";
import { ShellTree } from "./ShellTree";
import type { ShellTreeActionProps } from "./ShellTreeActions";

export function useShellTree({
  library, sticks, selectedNode, onSelectNode, view, onViewChange, actions, fromStick = false, expansion, onExpansionChange,
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
}): { tree: ReactNode; intakeQueue: IntakeQueue } {
  const [intakeQueue, setIntakeQueue] = useState<IntakeQueue>("intake");
  const [queueChosen, setQueueChosen] = useState(false);

  const open = useCallback((row: ShellTreeRow) => {
    setQueueChosen(row.queue !== undefined);
    if (row.queue !== undefined) setIntakeQueue(row.queue);
    else if (row.node !== undefined) onSelectNode(row.node);
    onViewChange(shellViewForRow(row.kind));
  }, [onSelectNode, onViewChange]);

  const selectedId = view === "intake" && queueChosen ? shellTreeQueueId(intakeQueue) : selectedNode?.id ?? null;
  const treeActions = useMemo(
    () => (actions ? { ...actions, canDropToDevicePlaylists: fromStick } : undefined),
    [actions, fromStick],
  );
  const collapsed = useMemo(() => (expansion ? shellCollapsed(expansion) : undefined), [expansion]);
  const folderOpen = useCallback((id: string, isOpen: boolean) => {
    if (expansion && onExpansionChange) onExpansionChange(setShellFolderOpen(expansion, id, isOpen));
  }, [expansion, onExpansionChange]);
  const tree = (
    <ShellTree
      library={library} sticks={sticks} selectedId={selectedId} onOpen={open}
      actions={treeActions} selectedNode={selectedNode} collapsed={collapsed} onFolderOpen={folderOpen}
    />
  );
  return { tree, intakeQueue };
}
