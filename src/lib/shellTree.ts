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
 * Pure functions over the Library's flat node array and each mounted
 * Stick's outline (`stickOutline`, read without writing to the Stick), so
 * the shape is tested without a backend. `ShellTree.tsx` draws the rows.
 *
 * Rows are appended one at a time, never spread into a call: a Library or
 * a Stick with hundreds of thousands of playlists would overflow the stack.
 */
import type { StickOutline, TreeNode } from "@/ipc/types";
import { deviceNodeId } from "./deviceLibrary";
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
  /** The name of one of a Stick's two libraries, when they differ. Not a place. */
  | "stickLibrary"
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
  /** A Stick's row: Stick Protection, once known (wbx-45.11). */
  protection?: "protected" | "unlocked";
}

/** A Set plan as the tree lists it. Nothing produces these yet (ticket #76). */
export interface ShellSetPlan {
  id: string;
  name: string;
  count?: number;
}

/** A mounted Stick, and its outline once read (null until then). */
export interface ShellStick {
  path: string;
  name: string;
  outline: StickOutline | null;
  /** Stick Protection, once read (wbx-45.11). */
  protection?: "protected" | "unlocked";
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
 * Appends the rows for a run of folders and lists to `out`, skipping what
 * sits under a folder the DJ closed. `base` is the depth of the run's top
 * level in the flat array.
 */
function nested(
  out: ShellTreeRow[],
  nodes: readonly TreeNode[],
  base: number,
  collapsed: ReadonlySet<string>,
  kindOf: (node: TreeNode) => ShellTreeRowKind,
  isFolder: (node: TreeNode) => boolean,
): void {
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
}

const LIBRARY_NAMES: Record<StickOutline["libraries"][number]["format"], string> = {
  deviceLibrary: "Device Library",
  oneLibrary: "OneLibrary",
};

type StickLibrary = StickOutline["libraries"][number];

/** Whether two libraries list the same playlists, in the same tree, with the same counts. */
function sameLists(a: StickLibrary, b: StickLibrary): boolean {
  if (a.nodes.length !== b.nodes.length) return false;
  return a.nodes.every((node, i) => {
    const other = b.nodes[i];
    return other !== undefined && other.name === node.name && other.folder === node.folder &&
      other.depth === node.depth && other.count === node.count;
  });
}

/** A Stick library's nodes as the Library view opens them (see `deviceLibrary.ts`). */
function stickNodes(path: string, library: StickLibrary): TreeNode[] {
  const out: TreeNode[] = [];
  for (const node of library.nodes) {
    out.push({
      id: deviceNodeId({ path, format: library.format, role: "node", id: node.id }),
      name: node.name,
      kind: node.folder ? "deviceFolder" : "devicePlaylist",
      // Below the Stick, its library, and the library's Playlists heading,
      // as the old Devices tree lays them out.
      depth: node.depth + 3,
      childCount: node.count,
    });
  }
  return out;
}

function stickRows(out: ShellTreeRow[], stick: ShellStick, collapsed: ReadonlySet<string>): void {
  const id = `device:${stick.path}`;
  out.push({
    id, kind: "stick", name: stick.name, depth: 0, node: { id, name: stick.name, kind: "device", depth: 0 },
    ...(stick.protection === undefined ? {} : { protection: stick.protection }),
  });
  const outline = stick.outline;
  if (outline === null) return;
  if (outline.pending) {
    out.push(empty(id, "A write to this Stick did not finish"));
    return;
  }
  const lists = (library: StickLibrary) => {
    nested(
      out, stickNodes(stick.path, library), 3, collapsed,
      (node) => (node.kind === "deviceFolder" ? "stickFolder" : "stickPlaylist"),
      (node) => node.kind === "deviceFolder",
    );
  };
  const [first, second] = outline.libraries;
  // One list when there is one library, or both list the same playlists (a
  // Stick this app or rekordbox wrote); each under its own name otherwise,
  // since the DJ can change one without the other.
  if (first === undefined || (second === undefined && first.nodes.length === 0) ||
      (second !== undefined && first.nodes.length === 0 && second.nodes.length === 0)) {
    out.push(empty(id, "No playlists"));
    return;
  }
  if (second === undefined || sameLists(first, second)) {
    lists(first);
    return;
  }
  for (const library of outline.libraries) {
    const libraryId = `${id}:library:${library.format}`;
    out.push({ id: libraryId, kind: "stickLibrary", name: LIBRARY_NAMES[library.format], depth: 0 });
    if (library.nodes.length === 0) out.push(empty(libraryId, "No playlists"));
    else lists(library);
  }
}

/**
 * The tree's rows, top to bottom.
 *
 * `library` is the Library's tree as the backend sends it; anything in it
 * that is not All Tracks or a playlist (Histories, the Explorer, Related
 * Tracks, the Tag List) is not part of this tree and is passed over.
 * Folders are open unless their id is in `collapsed`: the open and closed
 * state the Library's nodes carry is the old tree's and is not read.
 */
export function shellTreeRows({ library, sticks = [], setPlans = [], collapsed = new Set(), queueCounts }: {
  library: readonly TreeNode[];
  /** What waits in Intake and Problems (wbx-62); `null` for a count not known. */
  queueCounts?: { intake: number | null; problems: number | null } | null | undefined;
  sticks?: readonly ShellStick[];
  setPlans?: readonly ShellSetPlan[];
  collapsed?: ReadonlySet<string>;
}): ShellTreeRow[] {
  const out: ShellTreeRow[] = [heading("library", "Library")];

  const all = library.find((node) => node.kind === "allTracks");
  if (all) {
    out.push({
      id: all.id, kind: "allTracks", name: all.name, depth: 0, node: all,
      ...(all.childCount !== undefined ? { count: all.childCount } : {}),
    });
  }
  const intakeCount = queueCounts?.intake ?? null;
  const problemsCount = queueCounts?.problems ?? null;
  out.push({ id: SHELL_TREE_INTAKE_ID, kind: "intake", name: "Intake", depth: 0, queue: "intake", ...(intakeCount !== null ? { count: intakeCount } : {}) });
  out.push({ id: SHELL_TREE_PROBLEMS_ID, kind: "problems", name: "Problems", depth: 0, queue: "problems", ...(problemsCount !== null ? { count: problemsCount } : {}) });

  out.push(heading("playlists", "Playlists"));
  const lists: TreeNode[] = [];
  let top = Number.POSITIVE_INFINITY;
  for (const node of library) {
    if (node.kind !== "folder" && node.kind !== "playlist" && node.kind !== "smartPlaylist") continue;
    lists.push(node);
    if (node.depth < top) top = node.depth;
  }
  nested(
    out, lists, top, collapsed,
    (node) => (node.kind === "folder" ? "folder" : node.kind === "smartPlaylist" ? "smartPlaylist" : "playlist"),
    (node) => node.kind === "folder",
  );

  out.push(heading("setPlans", "Set plans"));
  if (setPlans.length === 0) out.push(empty("setPlans", "No Set plans yet"));
  for (const plan of setPlans) {
    out.push({
      id: plan.id, kind: "setPlan", name: plan.name, depth: 0,
      ...(plan.count !== undefined ? { count: plan.count } : {}),
    });
  }

  for (const stick of sticks) stickRows(out, stick, collapsed);
  return out;
}

/** Whether a row is a place that can be selected. */
export function isShellTreePlace(kind: ShellTreeRowKind): boolean {
  return kind !== "heading" && kind !== "empty" && kind !== "stickLibrary";
}

/** The view a selected row opens in: its queue in Intake, a Stick in the Stick view, a list in the Library. */
export function shellViewForRow(kind: ShellTreeRowKind): ShellView {
  if (kind === "intake" || kind === "problems") return "intake";
  if (kind === "stick") return "stick";
  return "library";
}
