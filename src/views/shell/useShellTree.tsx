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
import { shellTreeQueueId, shellViewForRow, type IntakeQueue, type ShellTreeRow } from "@/lib/shellTree";
import type { ShellView } from "@/lib/shellViews";
import { ShellTree } from "./ShellTree";

export function useShellTree({ library, sticks, readSticks, selectedNode, onSelectNode, view, onViewChange, onReadStick }: {
  /** The Library's tree, as the backend sends it. */
  library: readonly TreeNode[];
  /** The mounted Sticks' nodes, as `useDeviceLibraries` lays them out. */
  sticks: readonly TreeNode[];
  /** The paths of the Sticks whose libraries have been read. */
  readSticks: ReadonlySet<string>;
  selectedNode: TreeNode | null;
  onSelectNode: (node: TreeNode) => void;
  view: ShellView;
  onViewChange: (view: ShellView) => void;
  onReadStick: (node: TreeNode) => void;
}): { tree: ReactNode; intakeQueue: IntakeQueue } {
  const [intakeQueue, setIntakeQueue] = useState<IntakeQueue>("intake");
  const [queueChosen, setQueueChosen] = useState(false);
  const nodes = useMemo(() => [...library, ...sticks], [library, sticks]);

  const open = useCallback((row: ShellTreeRow) => {
    setQueueChosen(row.queue !== undefined);
    if (row.queue !== undefined) setIntakeQueue(row.queue);
    else if (row.node !== undefined) onSelectNode(row.node);
    onViewChange(shellViewForRow(row.kind));
  }, [onSelectNode, onViewChange]);

  const selectedId = view === "intake" && queueChosen ? shellTreeQueueId(intakeQueue) : selectedNode?.id ?? null;
  const tree = <ShellTree nodes={nodes} selectedId={selectedId} onOpen={open} onReadStick={onReadStick} readSticks={readSticks} />;
  return { tree, intakeQueue };
}
