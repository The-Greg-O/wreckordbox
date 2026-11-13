/**
 * The tree when the shell is too narrow to draw it (wbx-45.21;
 * docs/design/screens/library "Sizes": "no tree (the playlist name becomes
 * a breadcrumb)"). At 800 CSS px and below the open list's heading is a
 * breadcrumb, and "Go to the Tree" (the breadcrumb, the palette, Ctrl+T)
 * draws the shell's own tree over the view; at full width it moves the
 * keyboard to the tree. No row in `BINDINGS`, as the view keys: the key is
 * the shell's own.
 */
import type { TreeNode } from "@/ipc/types";
import { chordMatches, isTyping, type Action, type FocusTarget, type KeyChord, type Platform } from "./shortcuts";

export const GO_TO_TREE: { action: Action; label: string; chord: KeyChord } = {
  action: "goToTree", label: "Go to the Tree", chord: { key: "t", metaKey: true },
};

type KeyLike = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">;

/** Whether a key press asks for the tree: Ctrl+T, but not in a field or a dialog. */
export function treeKeyAction(event: KeyLike, platform: Platform, focus?: FocusTarget | null): boolean {
  if (platform.mac ? event.ctrlKey : event.metaKey) return false;
  if (isTyping(focus)) return false;
  if (focus?.closest?.('[role="dialog"], [role="alertdialog"], dialog')) return false;
  const chord: KeyChord = {
    key: event.key, code: event.code, ctrlKey: event.ctrlKey, metaKey: event.metaKey, shiftKey: event.shiftKey, altKey: event.altKey,
  };
  return chordMatches(GO_TO_TREE.chord, chord, platform);
}

/**
 * The breadcrumb of the open list: the folders above it in the tree (which
 * is flat, each node with its depth), then its name. A list the tree does
 * not hold (All Tracks, a Stick's playlist) is its own name alone.
 */
export function breadcrumbOf(tree: readonly TreeNode[], open: { id: string; name: string } | null): string[] {
  if (open === null) return ["Collection"];
  const folders: { depth: number; name: string }[] = [];
  for (const node of tree) {
    while (folders.length > 0 && folders[folders.length - 1]!.depth >= node.depth) folders.pop();
    if (node.id === open.id) return [...folders.map((folder) => folder.name), node.name];
    if (node.kind === "folder") folders.push({ depth: node.depth, name: node.name });
  }
  return [open.name];
}
