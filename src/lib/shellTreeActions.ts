/**
 * What each row of the view shell's tree offers (wbx-45.4): its menu, what a
 * drop on it does, whether it renames or moves, and where a moved list
 * lands. Pure, so the decisions are tested without a window; the actions
 * themselves are the old tree's handlers, run by `ShellTreeActions.tsx`.
 *
 * The menus are the old tree's (`treeMenu`, `deviceTreeMenu`), opened on the
 * same nodes: a Library list on its own node, the Playlists heading on the
 * collection node, a Stick's lists on their device node ids. The shell has no
 * row for a Stick library's Playlists heading, so its menu (Create New
 * Playlist and Folder on the Stick) is the Stick row's, or the library
 * label's when the Stick's two libraries differ.
 *
 * Safety: a Stick is offered nothing but Eject until its outline has been
 * read and says no write is unfinished, so no action from the tree can be
 * the thing that settles an interrupted export (wbx-45.5).
 */
import type { DeviceFormat, TreeNode } from "@/ipc/types";
import type { TreeExpansion } from "./session";
import { deviceNodeId } from "./deviceLibrary";
import type { ShellStick, ShellTreeRow } from "./shellTree";
import { childrenOf, containerOf, subtreeIds } from "./tree";

export type ShellTreeMenu =
  /** The old tree's menu for a Library list, or the Playlists heading's. */
  | { kind: "tree"; menu: "playlist" | "smartPlaylist" | "folder" | "collection"; node: TreeNode }
  /** The old Devices tree's menu for a Stick's list or Playlists heading; `eject` on the Stick row. */
  | { kind: "stick"; menu: "devicePlaylists" | "deviceFolder" | "devicePlaylist"; node: TreeNode; eject?: TreeNode }
  /** A Stick that can only be ejected from here. */
  | { kind: "eject"; eject: TreeNode };

/** What a drop on a row does. */
export type ShellDrop =
  /** Tracks added to a Library playlist. */
  | "tracks"
  /** Tracks added to a Stick's playlist. */
  | "stickTracks"
  /** Tracks exported to a Stick. */
  | "exportTracks"
  /** A Library playlist or folder exported to a Stick. */
  | "exportNode"
  /** Files from outside the window, imported into the Library there. */
  | "files";

/** What is in hand during a drag. */
export interface ShellDrag {
  /** Tracks from a list. */
  tracks: boolean;
  /** Those tracks can be exported to a Stick (none is a loose file). */
  toStick: boolean;
  /** A Library playlist or folder. */
  node: boolean;
  /** Files from outside the window. */
  files: boolean;
}

const STICK_ROW_PREFIX = "device:";

function stickOf(row: ShellTreeRow, sticks: readonly ShellStick[]): ShellStick | undefined {
  return sticks.find((stick) => row.id === `${STICK_ROW_PREFIX}${stick.path}` ||
    row.id.startsWith(`${STICK_ROW_PREFIX}${stick.path}:library:`));
}

/** The Stick is read and its last write finished, so it may be written to. */
function ready(stick: ShellStick | undefined): stick is ShellStick & { outline: NonNullable<ShellStick["outline"]> } {
  return stick?.outline != null && !stick.outline.pending;
}

/** The node a Stick library's Playlists heading had in the old Devices tree. */
function playlistsHeading(path: string, format: DeviceFormat): TreeNode {
  return { id: deviceNodeId({ path, format, role: "playlists", id: "0" }), name: "Playlists", kind: "devicePlaylists", depth: 2 };
}

/** The Playlists heading's menu: the old tree's, on the collection node. */
export function playlistsMenu(library: readonly TreeNode[]): ShellTreeMenu | null {
  const node = library.find((n) => n.kind === "collection");
  return node ? { kind: "tree", menu: "collection", node } : null;
}

/** The menu a right-click (or the menu key) opens on a row, if any. */
export function shellRowMenu(row: ShellTreeRow, sticks: readonly ShellStick[]): ShellTreeMenu | null {
  const node = row.node;
  switch (row.kind) {
    case "playlist":
    case "smartPlaylist":
    case "folder":
      return node ? { kind: "tree", menu: row.kind, node } : null;
    case "stickPlaylist":
    case "stickFolder":
      return node && (node.kind === "devicePlaylist" || node.kind === "deviceFolder")
        ? { kind: "stick", menu: node.kind, node }
        : null;
    case "stick": {
      const stick = stickOf(row, sticks);
      const eject = node ?? { id: row.id, name: row.name, kind: "device", depth: 0 };
      if (!ready(stick)) return { kind: "eject", eject };
      const [first, second] = stick.outline.libraries;
      // Which library a new playlist goes in is only clear with one list shown.
      const one = first !== undefined && (second === undefined || shownAsOne(stick));
      return one ? { kind: "stick", menu: "devicePlaylists", node: playlistsHeading(stick.path, first.format), eject } : { kind: "eject", eject };
    }
    case "stickLibrary": {
      const stick = stickOf(row, sticks);
      const format = row.id.slice(row.id.lastIndexOf(":") + 1) as DeviceFormat;
      return ready(stick) ? { kind: "stick", menu: "devicePlaylists", node: playlistsHeading(stick.path, format) } : null;
    }
    default:
      return null;
  }
}

/** Whether the tree lists the Stick's libraries as one (see `shellTreeRows`). */
function shownAsOne(stick: ShellStick): boolean {
  const [a, b] = stick.outline?.libraries ?? [];
  if (a === undefined || b === undefined) return true;
  return a.nodes.length === b.nodes.length && a.nodes.every((node, i) => {
    const other = b.nodes[i];
    return other !== undefined && other.name === node.name && other.folder === node.folder &&
      other.depth === node.depth && other.count === node.count;
  });
}

/** What dropping `drag` on `row` would do, or null where the row takes no such drop. */
export function shellRowDrop(row: ShellTreeRow, sticks: readonly ShellStick[], drag: ShellDrag): ShellDrop | null {
  if (row.kind === "stick") {
    if (!ready(stickOf(row, sticks))) return null;
    if (drag.node) return "exportNode";
    if (drag.tracks && drag.toStick) return "exportTracks";
    return null;
  }
  if (drag.tracks) {
    if (row.kind === "playlist") return "tracks";
    if (row.kind === "stickPlaylist") return "stickTracks";
    return null;
  }
  if (drag.files && (row.kind === "allTracks" || row.kind === "playlist" || row.kind === "folder")) return "files";
  return null;
}

/** Library lists and a Stick's own lists rename in place. */
export function shellRowRenamable(row: ShellTreeRow): boolean {
  return row.kind === "playlist" || row.kind === "smartPlaylist" || row.kind === "folder" ||
    row.kind === "stickPlaylist" || row.kind === "stickFolder";
}

/** Only the Library's lists move within the tree. */
export function shellRowMovable(row: ShellTreeRow): boolean {
  return row.kind === "playlist" || row.kind === "smartPlaylist" || row.kind === "folder";
}

/**
 * Where `node` lands when dropped on `target` (`into` a folder's middle, or
 * `above` / `below` it), as the old tree computes it: the parent's id
 * (`TREE_ROOT` for the top) and the index among that parent's children,
 * counted with the node lifted out. Null where it cannot land.
 */
export function moveDestination(
  nodes: readonly TreeNode[],
  node: TreeNode,
  target: TreeNode,
  edge: "above" | "below" | "into",
): { parent: string; index: number } | null {
  if (subtreeIds(nodes, node).has(target.id)) return null;
  if (target.kind !== "playlist" && target.kind !== "folder") return null;
  if (edge === "into") {
    if (target.kind !== "folder") return null;
    return { parent: target.id, index: childrenOf(nodes, target.id).length };
  }
  const parent = containerOf(nodes, target);
  const siblings = childrenOf(nodes, parent).filter((n) => n.id !== node.id);
  const at = siblings.findIndex((n) => n.id === target.id);
  if (at < 0) return null;
  return { parent, index: edge === "below" ? at + 1 : at };
}

/** The folders closed in the session's tree state. */
export function shellCollapsed(expansion: TreeExpansion): ReadonlySet<string> {
  return new Set(expansion.collapsed);
}

/** The session's tree state with one folder opened or closed; the same value when nothing changes. */
export function setShellFolderOpen(expansion: TreeExpansion, id: string, open: boolean): TreeExpansion {
  const isClosed = expansion.collapsed.includes(id);
  const isOpened = expansion.expanded.includes(id);
  if (open ? !isClosed && isOpened : isClosed && !isOpened) return expansion;
  return open
    ? { collapsed: expansion.collapsed.filter((c) => c !== id), expanded: isOpened ? expansion.expanded : [...expansion.expanded, id] }
    : { collapsed: isClosed ? expansion.collapsed : [...expansion.collapsed, id], expanded: expansion.expanded.filter((e) => e !== id) };
}


const count = (n: number, one: string) => `${n} ${one}${n === 1 ? "" : "s"}`;

/**
 * The question asked before a Library playlist or folder is deleted from
 * the shell's tree (wbx-45.4 review, finding 8): what goes, with every
 * playlist and folder under a folder and how many tracks they list.
 */
export function deleteQuestion(nodes: readonly TreeNode[], node: TreeNode): string {
  const keep = "The tracks stay in the Library, and Undo brings";
  if (node.kind !== "folder") {
    return `Delete the playlist “${node.name}”? It lists ${count(node.childCount ?? 0, "track")}. ${keep} the playlist back.`;
  }
  const inside = subtreeIds(nodes, node);
  let playlists = 0;
  let folders = 0;
  let tracks = 0;
  for (const n of nodes) {
    if (n.id === node.id || !inside.has(n.id)) continue;
    if (n.kind === "folder") folders += 1;
    else {
      playlists += 1;
      tracks += n.childCount ?? 0;
    }
  }
  if (playlists + folders === 0) return `Delete the empty folder “${node.name}”? Undo brings it back.`;
  const what = [playlists > 0 ? count(playlists, "playlist") : null, folders > 0 ? count(folders, "folder") : null]
    .filter((part) => part !== null).join(" and ");
  return `Delete the folder “${node.name}” and the ${what} in it? They list ${count(tracks, "track")}. ${keep} the folder back.`;
}
