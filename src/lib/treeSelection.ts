import type { TreeNode } from "@/ipc/types";

/** The kinds the Library's own playlist tree answers, and so can take away. */
const LIBRARY_KINDS: ReadonlySet<TreeNode["kind"]> = new Set(["playlist", "smartPlaylist", "folder", "history"]);

/**
 * The open node, once the Library's tree has been read again (wbx-w41).
 *
 * A node the tree still holds is followed as the tree now has it, so a rename
 * reaches the list's heading. One the tree no longer holds (deleted here, by
 * Undo, or by another client) is left for All Tracks, and `gone` names it so
 * the window can say so: a backend asked for a playlist it no longer has
 * answers with the whole collection, and the list then went on naming the
 * deleted playlist over the collection's rows. Nodes the Library tree never
 * holds (a Stick's, the Tag List, a folder on disk) are not this tree's to
 * judge and are kept.
 */
export function selectionInTree(
  selected: TreeNode | null,
  tree: readonly TreeNode[],
): { node: TreeNode | null; gone: string | null } {
  if (selected === null || !LIBRARY_KINDS.has(selected.kind)) return { node: selected, gone: null };
  const now = tree.find((node) => node.id === selected.id && node.kind === selected.kind);
  if (now) return { node: now.name === selected.name ? selected : now, gone: null };
  return { node: tree.find((node) => node.kind === "allTracks") ?? tree[0] ?? null, gone: selected.name };
}
