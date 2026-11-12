/**
 * The Library view's second list: a target playlist beside the list the DJ
 * is reading from (wbx-59, `docs/design/screens/library/`).
 *
 * Pure: which playlists can be a target, the keys, the order arithmetic
 * behind a reorder by key or by drag, the swap, and the length drawn in the
 * list's heading. The list itself is `src/views/shell/TargetList.tsx`; the
 * state the window keeps is `src/views/shell/useTargetList.tsx`.
 *
 * Adding and reordering are the Library's existing playlist edits (Add
 * Tracks to Playlist, Reorder Playlist), so Library Protection and Undo
 * apply to them exactly as they do anywhere else.
 */
import type { TreeNode } from "@/ipc/types";
import { chordMatches, isTyping, OWN_ACTIVATION, type Action, type FocusTarget, type KeyChord, type Platform } from "./shortcuts";

/** The second list's commands, their names in the command palette, and their keys. */
export const TARGET_ACTIONS: readonly { action: Action; label: string; chord: KeyChord }[] = [
  { action: "toggleTargetList", label: "Show or Hide the Second List", chord: { key: "\\", metaKey: true } },
  { action: "addToTarget", label: "Add Selected Tracks to the Target", chord: { key: "Enter" } },
  { action: "moveTargetUp", label: "Move Up in the Target", chord: { key: "ArrowUp", altKey: true } },
  { action: "moveTargetDown", label: "Move Down in the Target", chord: { key: "ArrowDown", altKey: true } },
  { action: "swapTargetLists", label: "Swap Source and Target", chord: { key: "\\", metaKey: true, shiftKey: true } },
];

type KeyLike = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">;

/**
 * The second list's command a key press asks for, or `null`: some other
 * key, a field or a dialog has the keyboard, or Enter is pressed on a
 * control that owns it (an Energy pip, a button, the Track Filter bar).
 */
export function targetKeyAction(event: KeyLike, platform: Platform, focus?: FocusTarget | null): Action | null {
  // The other platform's modifier is never this platform's command key.
  if (platform.mac ? event.ctrlKey : event.metaKey) return null;
  if (isTyping(focus)) return null;
  if (focus?.closest?.('[role="dialog"], [role="alertdialog"], dialog')) return null;
  if (event.key === "Enter" && focus?.closest?.(`[${OWN_ACTIVATION}], button, a[href]`)) return null;
  const chord: KeyChord = {
    key: event.key, code: event.code, ctrlKey: event.ctrlKey, metaKey: event.metaKey, shiftKey: event.shiftKey, altKey: event.altKey,
  };
  return TARGET_ACTIONS.find((entry) => chordMatches(entry.chord, chord, platform))?.action ?? null;
}

/** "1h18m", "42m": a playlist's length as the design's heading draws it. */
export function formatTargetLength(seconds: number): string {
  const minutes = Math.floor(Math.max(0, seconds) / 60);
  const hours = Math.floor(minutes / 60);
  if (hours === 0) return `${minutes}m`;
  return `${hours}h${String(minutes % 60).padStart(2, "0")}m`;
}

export interface TargetCandidate {
  id: string;
  name: string;
  /** The name under its folders, so two playlists of one name can be told apart. */
  path: string;
}

/** Every playlist of the Library that can be a target, in the tree's order. */
export function targetCandidates(tree: readonly TreeNode[]): TargetCandidate[] {
  const out: TargetCandidate[] = [];
  // The folders open above the node being read, by depth.
  const folders: { depth: number; name: string }[] = [];
  for (const node of tree) {
    while (folders.length > 0 && folders[folders.length - 1]!.depth >= node.depth) folders.pop();
    if (node.kind === "folder") folders.push({ depth: node.depth, name: node.name });
    if (node.kind !== "playlist") continue;
    out.push({ id: node.id, name: node.name, path: [...folders.map((folder) => folder.name), node.name].join(" / ") });
  }
  return out;
}

/**
 * Why a node cannot be the target, or `null` when it can. Only a plain
 * playlist of the Library has an order of its own that tracks are added to.
 *
 * A Stick's playlist is refused outright: writing to a Stick must go
 * through Stick Protection (wbx-45.11), which is not built yet, and the
 * second list makes no Stick write of its own.
 */
export function targetRefusal(node: TreeNode | null | undefined): string | null {
  if (!node) return "Choose a playlist as the target.";
  switch (node.kind) {
    case "playlist":
      return null;
    case "devicePlaylist":
    case "deviceFolder":
    case "deviceAllTracks":
    case "devicePlaylists":
    case "deviceLibrary":
    case "device":
      return "A Stick's playlist cannot be the target: writing to a Stick needs Stick Protection, which is not built yet.";
    case "smartPlaylist":
      return "An intelligent playlist cannot be the target: its rule chooses its tracks.";
    case "folder":
      return "A folder cannot be the target: choose a playlist in it.";
    default:
      return "Only a playlist can be the target, not All Tracks or a list the Library keeps itself.";
  }
}

/** One row moved one place up (`-1`) or down (`1`), and where it is now; `null` past either end. */
export function moveInOrder(order: readonly string[], index: number, delta: -1 | 1): { order: string[]; index: number } | null {
  const to = index + delta;
  if (index < 0 || index >= order.length || to < 0 || to >= order.length) return null;
  const next = [...order];
  [next[index], next[to]] = [next[to]!, next[index]!];
  return { order: next, index: to };
}

/**
 * The row at `from` dragged and dropped before the row at `before` (the
 * length of the list: after every row). `null` when that is where it is.
 */
export function dropInOrder(order: readonly string[], from: number, before: number): string[] | null {
  if (from < 0 || from >= order.length) return null;
  const at = Math.max(0, Math.min(order.length, before));
  if (at === from || at === from + 1) return null;
  const next = [...order];
  const [moved] = next.splice(from, 1);
  next.splice(at > from ? at - 1 : at, 0, moved!);
  return next;
}

/**
 * The swap: the target opens in the main list and the playlist the main
 * list showed becomes the target, so the target can be worked on at full
 * width. Refused, with the reason, when the main list does not show a
 * playlist that can be a target.
 */
export function swapLists(
  source: TreeNode | null | undefined, target: TreeNode | null | undefined,
): { open: TreeNode; target: TreeNode } | { refusal: string } {
  if (!target) return { refusal: "There is no target to swap with: choose a target playlist first." };
  const refused = targetRefusal(source);
  if (refused !== null) return { refusal: `Cannot swap: ${refused}` };
  return { open: target, target: source! };
}
