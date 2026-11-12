/**
 * The view shell's tree (ticket #52): everything the DJ can open, in one
 * list.
 *
 * Four groups under headings, top to bottom: the Library (All Tracks, and
 * the Intake and Problems queues), the playlists (every folder, playlist and
 * intelligent playlist at any depth), Set plans, and one group per mounted
 * Stick with that Stick's playlists. There are no sections to switch
 * between as the old tree's rail had: all of it is in view at once.
 *
 * Pure functions over the flat node array the window already holds (the
 * Library's tree followed by the Devices section, see `deviceLibrary.ts`),
 * so the shape is tested without a backend. `ShellTree.tsx` draws the rows.
 */
import type { TreeNode } from "@/ipc/types";
import { devicePath } from "./devices";
import type { ShellView } from "./shellViews";

/** The two queues the Intake view shows: no Prep colour, and Prep colour Problem. */
export type IntakeQueue = "intake" | "problems";

export type ShellTreeRowKind =
  /** A group's name. Not a place: nothing opens. */
  | "heading"
  /** A line saying a group has nothing in it. Not a place either. */
  | "empty"
  | "allTracks" | "intake" | "problems"
  | "folder" | "playlist" | "smartPlaylist"
  | "setPlan"
  /** A mounted Stick: its group's heading, and the way into the Stick view. */
  | "stick"
  | "stickFolder" | "stickPlaylist";

export interface ShellTreeRow {
  id: string;
  kind: ShellTreeRowKind;
  /** In English; a heading, a queue and an empty line are drawn through the translation. */
  name: string;
  /** Levels below the group's heading: 0 sits directly under it. */
  depth: number;
  /** How many tracks, where the Library says. */
  count?: number;
  /** The Library's or the Stick's own node this row opens. */
  node?: TreeNode;
  /** The queue this row opens in the Intake view. */
  queue?: IntakeQueue;
  /** A folder: it can be closed and opened. */
  branch?: true;
  open?: boolean;
}

/** A Set plan as the tree lists it. Nothing produces these yet (ticket #76). */
export interface ShellSetPlan {
  id: string;
  name: string;
  count?: number;
}

export const SHELL_TREE_INTAKE_ID = "shell:intake";
export const SHELL_TREE_PROBLEMS_ID = "shell:problems";

/** The row id of a queue, for showing it selected. */
export function shellTreeQueueId(queue: IntakeQueue): string {
  return queue === "intake" ? SHELL_TREE_INTAKE_ID : SHELL_TREE_PROBLEMS_ID;
}

const heading = (id: string, name: string): ShellTreeRow => ({ id: `shell:heading:${id}`, kind: "heading", name, depth: 0 });
const empty = (id: string, name: string): ShellTreeRow => ({ id: `shell:empty:${id}`, kind: "empty", name, depth: 0 });

/**
 * Rows for a run of folders and lists, skipping what sits under a folder
 * the DJ closed. `base` is the depth of the run's top level in the flat
 * array.
 */
function nested(
  nodes: readonly TreeNode[],
  base: number,
  collapsed: ReadonlySet<string>,
  kindOf: (node: TreeNode) => ShellTreeRowKind,
  isFolder: (node: TreeNode) => boolean,
): ShellTreeRow[] {
  const out: ShellTreeRow[] = [];
  // The depth of the closed folder being skipped under, if any.
  let closedAt: number | null = null;
  for (const node of nodes) {
    if (closedAt !== null) {
      if (node.depth > closedAt) continue;
      closedAt = null;
    }
    const folder = isFolder(node);
    const open = !collapsed.has(node.id);
    out.push({
      id: node.id,
      kind: kindOf(node),
      name: node.name,
      depth: Math.max(0, node.depth - base),
      node,
      ...(node.childCount !== undefined && !folder ? { count: node.childCount } : {}),
      ...(folder ? { branch: true as const, open } : {}),
    });
    if (folder && !open) closedAt = node.depth;
  }
  return out;
}

/**
 * The tree's rows, top to bottom.
 *
 * `nodes` is the Library's tree and the Devices section in one flat array;
 * anything else in it (Histories, the Explorer, Related Tracks, the Tag
 * List) is not part of this tree and is passed over. Folders are open unless
 * their id is in `collapsed`: the open and closed state the Library's nodes
 * carry is the old tree's and is not read. A Stick not yet read has no line
 * under it; one read with no playlists says so.
 */
export function shellTreeRows({ nodes, setPlans = [], collapsed = new Set(), readSticks = new Set() }: {
  nodes: readonly TreeNode[];
  setPlans?: readonly ShellSetPlan[];
  collapsed?: ReadonlySet<string>;
  /**
   * The paths of the Sticks whose libraries have been read, so one that
   * holds none (never written) can be told from one not read yet.
   */
  readSticks?: ReadonlySet<string>;
}): ShellTreeRow[] {
  const out: ShellTreeRow[] = [heading("library", "Library")];

  const all = nodes.find((node) => node.kind === "allTracks");
  if (all) {
    out.push({
      id: all.id, kind: "allTracks", name: all.name, depth: 0, node: all,
      ...(all.childCount !== undefined ? { count: all.childCount } : {}),
    });
  }
  out.push({ id: SHELL_TREE_INTAKE_ID, kind: "intake", name: "Intake", depth: 0, queue: "intake" });
  out.push({ id: SHELL_TREE_PROBLEMS_ID, kind: "problems", name: "Problems", depth: 0, queue: "problems" });

  out.push(heading("playlists", "Playlists"));
  const lists = nodes.filter((node) => node.kind === "folder" || node.kind === "playlist" || node.kind === "smartPlaylist");
  const top = lists.reduce((least, node) => Math.min(least, node.depth), Number.POSITIVE_INFINITY);
  out.push(...nested(
    lists, top, collapsed,
    (node) => (node.kind === "folder" ? "folder" : node.kind === "smartPlaylist" ? "smartPlaylist" : "playlist"),
    (node) => node.kind === "folder",
  ));

  out.push(heading("setPlans", "Set plans"));
  if (setPlans.length === 0) out.push(empty("setPlans", "No Set plans yet"));
  for (const plan of setPlans) {
    out.push({
      id: plan.id, kind: "setPlan", name: plan.name, depth: 0,
      ...(plan.count !== undefined ? { count: plan.count } : {}),
    });
  }

  // Each mounted Stick, then the playlists of the first library read from
  // it. A Stick written by this app or by rekordbox holds the same playlists
  // in both of its libraries (Device Library and OneLibrary), so they are
  // listed once; the old Devices tree is where the two are told apart.
  for (let i = 0; i < nodes.length; i += 1) {
    const device = nodes[i];
    if (device?.kind !== "device") continue;
    out.push({ id: device.id, kind: "stick", name: device.name, depth: 0, node: device });
    const under: TreeNode[] = [];
    let libraries = 0;
    for (let j = i + 1; j < nodes.length; j += 1) {
      const node = nodes[j];
      if (node === undefined || node.depth <= device.depth) break;
      if (node.kind === "deviceLibrary") libraries += 1;
      if (libraries > 1) break;
      if (node.kind === "deviceFolder" || node.kind === "devicePlaylist") under.push(node);
    }
    const path = devicePath(device.id);
    if (libraries === 0 && (path === null || !readSticks.has(path))) continue;
    if (under.length === 0) {
      out.push(empty(device.id, "No playlists"));
      continue;
    }
    out.push(...nested(
      under, device.depth + 3, collapsed,
      (node) => (node.kind === "deviceFolder" ? "stickFolder" : "stickPlaylist"),
      (node) => node.kind === "deviceFolder",
    ));
  }
  return out;
}

/** Whether a row is a place that can be selected. */
export function isShellTreePlace(kind: ShellTreeRowKind): boolean {
  return kind !== "heading" && kind !== "empty";
}

/** The view a selected row opens in: its queue in Intake, a Stick in the Stick view, a list in the Library. */
export function shellViewForRow(kind: ShellTreeRowKind): ShellView {
  if (kind === "intake" || kind === "problems") return "intake";
  if (kind === "stick") return "stick";
  return "library";
}
