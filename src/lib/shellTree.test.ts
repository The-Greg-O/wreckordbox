import { describe, expect, it } from "vitest";

import type { TreeNode } from "@/ipc/types";
import { deviceNodeId } from "./deviceLibrary";
import { shellTreeRows, shellViewForRow, type ShellStick, type ShellTreeRow } from "./shellTree";

/** A Library tree as the backend sends it: flat, with a depth per node. */
const LIBRARY: TreeNode[] = [
  { id: "all", name: "All Tracks", kind: "allTracks", depth: 0, childCount: 412 },
  { id: "playlists", name: "Playlists", kind: "collection", depth: 0, expanded: true },
  { id: "f1", name: "CURRENT", kind: "folder", depth: 1, expanded: false },
  { id: "p1", name: "Melodic Vox", kind: "playlist", depth: 2, childCount: 30 },
  { id: "f2", name: "Deep", kind: "folder", depth: 2, expanded: false },
  { id: "f3", name: "Deeper", kind: "folder", depth: 3 },
  { id: "p2", name: "Bottom", kind: "playlist", depth: 4, childCount: 3 },
  { id: "s1", name: "Fresh 128s", kind: "smartPlaylist", depth: 2 },
  { id: "p3", name: "Loose", kind: "playlist", depth: 1, childCount: 0 },
  { id: "histories", name: "Histories", kind: "histories", depth: 0 },
  { id: "h1", name: "2026", kind: "history", depth: 1 },
  { id: "explorer", name: "Explorer", kind: "explorer", depth: 0 },
];

const STICK = "/run/media/dj/GIGSTICK";
type Outline = NonNullable<ShellStick["outline"]>;
type Library = Outline["libraries"][number];
const node = (id: number, parent: number, name: string, depth: number, count: number, folder = false) =>
  ({ id: String(id), parentId: String(parent), name, folder, depth, count });
/** The real stick's shape: a list, a folder holding one, at the top level. */
const PLAYLISTS = [node(1, 0, "Main", 0, 140), node(2, 0, "Sets", 0, 1, true), node(3, 2, "Techno", 1, 49)];
const lib = (format: Library["format"], nodes: Library["nodes"]): Library => ({ format, nodes });
const stick = (outline: Outline | null): ShellStick => ({ path: STICK, name: "GIGSTICK", outline });
const fromStick = (rows: ShellTreeRow[]) => rows.slice(rows.findIndex((row) => row.kind === "stick")).map(line);

/** A row as `kind depth name [count]`, for reading a whole tree at once. */
const line = (row: ShellTreeRow) =>
  `${row.kind} ${row.depth} ${row.name}${row.count === undefined ? "" : ` ${row.count}`}`;

describe("the shell's tree", () => {
  it("lists the Library's All Tracks, Intake and Problems, then every playlist at any depth, then Set plans", () => {
    expect(shellTreeRows({ library: LIBRARY }).map(line)).toEqual([
      "heading 0 Library",
      "allTracks 0 All Tracks 412",
      "intake 0 Intake",
      "problems 0 Problems",
      "heading 0 Playlists",
      "folder 0 CURRENT",
      "playlist 1 Melodic Vox 30",
      "folder 1 Deep",
      "folder 2 Deeper",
      "playlist 3 Bottom 3",
      "smartPlaylist 1 Fresh 128s",
      "playlist 0 Loose 0",
      "heading 0 Set plans",
      "empty 0 No Set plans yet",
    ]);
  });

  it("shows every folder open whatever the old tree had closed, and hides what is under one the DJ closes", () => {
    const rows = shellTreeRows({ library: LIBRARY, collapsed: new Set(["f2"]) });
    expect(rows.map((row) => row.name)).not.toContain("Deeper");
    expect(rows.map((row) => row.name)).not.toContain("Bottom");
    // What follows the closed folder at its own level is still there.
    expect(rows.map((row) => row.name)).toContain("Fresh 128s");
    expect(rows.find((row) => row.id === "f2")).toMatchObject({ branch: true, open: false });
    expect(rows.find((row) => row.id === "f1")).toMatchObject({ branch: true, open: true });
  });

  it("opens Library rows on the Library's own nodes, and Intake and Problems on a queue", () => {
    const rows = shellTreeRows({ library: LIBRARY });
    expect(rows.find((row) => row.kind === "allTracks")?.node).toBe(LIBRARY[0]);
    expect(rows.find((row) => row.name === "Bottom")?.node).toBe(LIBRARY[6]);
    expect(rows.find((row) => row.kind === "intake")).toMatchObject({ queue: "intake" });
    expect(rows.find((row) => row.kind === "problems")).toMatchObject({ queue: "problems" });
    expect(rows.find((row) => row.kind === "intake")?.node).toBeUndefined();
  });

  it("counts what waits in Intake and Problems when the counts are known (wbx-62)", () => {
    const counted = shellTreeRows({ library: LIBRARY, queueCounts: { intake: 23, problems: 4 } });
    expect(counted.find((row) => row.kind === "intake")?.count).toBe(23);
    expect(counted.find((row) => row.kind === "problems")?.count).toBe(4);
    const unknown = shellTreeRows({ library: LIBRARY, queueCounts: { intake: 2, problems: null } });
    expect(unknown.find((row) => row.kind === "problems")?.count).toBeUndefined();
    expect(shellTreeRows({ library: LIBRARY }).find((row) => row.kind === "intake")?.count).toBeUndefined();
  });

  it("lists Set plans once there are some", () => {
    const rows = shellTreeRows({ library: LIBRARY, setPlans: [{ id: "plan-1", name: "Friday peak hour", count: 16 }] });
    expect(rows.slice(-2).map(line)).toEqual(["heading 0 Set plans", "setPlan 0 Friday peak hour 16"]);
  });

  it("lists a mounted Stick with its playlists once when both of its libraries hold the same ones", () => {
    const rows = shellTreeRows({
      library: LIBRARY,
      sticks: [stick({ pending: false, libraries: [lib("deviceLibrary", PLAYLISTS), lib("oneLibrary", PLAYLISTS)] })],
    });
    expect(fromStick(rows)).toEqual([
      "stick 0 GIGSTICK",
      "stickPlaylist 0 Main 140",
      "stickFolder 0 Sets",
      "stickPlaylist 1 Techno 49",
    ]);
    expect(rows.find((row) => row.kind === "stick")?.node).toMatchObject({ id: `device:${STICK}`, kind: "device" });
    // A row opens that playlist of the Device Library, as the old tree's did.
    expect(rows.find((row) => row.name === "Techno")?.node).toMatchObject({
      id: deviceNodeId({ path: STICK, format: "deviceLibrary", role: "node", id: "3" }),
      kind: "devicePlaylist",
    });
    expect(shellTreeRows({ library: LIBRARY }).some((row) => row.kind === "stick")).toBe(false);
  });

  it("lists each library under its own name when they differ, so a playlist in only one is not lost", () => {
    const rows = shellTreeRows({
      library: LIBRARY,
      sticks: [stick({ pending: false, libraries: [lib("deviceLibrary", PLAYLISTS), lib("oneLibrary", [...PLAYLISTS, node(9, 0, "Only Here", 0, 2)])] })],
    });
    expect(fromStick(rows)).toEqual([
      "stick 0 GIGSTICK",
      "stickLibrary 0 Device Library",
      "stickPlaylist 0 Main 140",
      "stickFolder 0 Sets",
      "stickPlaylist 1 Techno 49",
      "stickLibrary 0 OneLibrary",
      "stickPlaylist 0 Main 140",
      "stickFolder 0 Sets",
      "stickPlaylist 1 Techno 49",
      "stickPlaylist 0 Only Here 2",
    ]);
    expect(rows.find((row) => row.name === "Only Here")?.node?.id).toBe(deviceNodeId({ path: STICK, format: "oneLibrary", role: "node", id: "9" }));
    // Every row has its own id, though the two libraries name theirs alike.
    expect(new Set(rows.map((row) => row.id)).size).toBe(rows.length);
  });

  it("shows OneLibrary's playlists when the Device Library is empty", () => {
    const rows = shellTreeRows({
      library: LIBRARY,
      sticks: [stick({ pending: false, libraries: [lib("deviceLibrary", []), lib("oneLibrary", PLAYLISTS)] })],
    });
    expect(fromStick(rows)).toEqual([
      "stick 0 GIGSTICK",
      "stickLibrary 0 Device Library",
      "empty 0 No playlists",
      "stickLibrary 0 OneLibrary",
      "stickPlaylist 0 Main 140",
      "stickFolder 0 Sets",
      "stickPlaylist 1 Techno 49",
    ]);
  });

  it("says nothing under a Stick not read yet, 'No playlists' under one read with none, and so when a write to it did not finish", () => {
    expect(fromStick(shellTreeRows({ library: LIBRARY, sticks: [stick(null)] }))).toEqual(["stick 0 GIGSTICK"]);
    expect(fromStick(shellTreeRows({ library: LIBRARY, sticks: [stick({ pending: false, libraries: [] })] }))).toEqual([
      "stick 0 GIGSTICK",
      "empty 0 No playlists",
    ]);
    expect(fromStick(shellTreeRows({ library: LIBRARY, sticks: [stick({ pending: false, libraries: [lib("oneLibrary", [])] })] }))).toEqual([
      "stick 0 GIGSTICK",
      "empty 0 No playlists",
    ]);
    expect(fromStick(shellTreeRows({ library: LIBRARY, sticks: [stick({ pending: true, libraries: [] })] }))).toEqual([
      "stick 0 GIGSTICK",
      "empty 0 A write to this Stick did not finish",
    ]);
  });

  it("builds a very large tree without running out of stack", () => {
    const many: TreeNode[] = [{ id: "all", name: "All Tracks", kind: "allTracks", depth: 0 }];
    for (let i = 0; i < 200_000; i += 1) many.push({ id: `p${i}`, name: `List ${i}`, kind: "playlist", depth: 1, childCount: 1 });
    const stickNodes = Array.from({ length: 200_000 }, (_, i) => node(i + 1, 0, `Stick list ${i}`, 0, 1));
    const rows = shellTreeRows({ library: many, sticks: [stick({ pending: false, libraries: [lib("deviceLibrary", stickNodes)] })] });
    expect(rows.length).toBeGreaterThan(400_000);
  });

  it("sends Intake and Problems to the Intake view, a Stick to the Stick view and the rest to the Library view", () => {
    expect(shellViewForRow("intake")).toBe("intake");
    expect(shellViewForRow("problems")).toBe("intake");
    expect(shellViewForRow("stick")).toBe("stick");
    for (const kind of ["allTracks", "folder", "playlist", "smartPlaylist", "setPlan", "stickPlaylist", "stickFolder"] as const) {
      expect(shellViewForRow(kind), kind).toBe("library");
    }
  });
});
