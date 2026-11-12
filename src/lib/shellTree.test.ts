import { describe, expect, it } from "vitest";

import type { TreeNode } from "@/ipc/types";
import { deviceNodeId } from "./deviceLibrary";
import { shellTreeRows, shellViewForRow, type ShellTreeRow } from "./shellTree";

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
const at = (format: "deviceLibrary" | "oneLibrary", role: "library" | "allTracks" | "playlists" | "node", id = "0") =>
  deviceNodeId({ path: STICK, format, role, id });
/** A Stick whose two libraries have been read, as `withDeviceLibraries` lays them out. */
const STICK_NODES: TreeNode[] = [
  { id: `device:${STICK}`, name: "GIGSTICK", kind: "device", depth: 0, lazy: true, expanded: false },
  { id: at("deviceLibrary", "library"), name: "Device Library", kind: "deviceLibrary", depth: 1 },
  { id: at("deviceLibrary", "allTracks"), name: "All Tracks", kind: "deviceAllTracks", depth: 2, childCount: 140 },
  { id: at("deviceLibrary", "playlists"), name: "Playlists", kind: "devicePlaylists", depth: 2 },
  { id: at("deviceLibrary", "node", "1"), name: "Main", kind: "devicePlaylist", depth: 3, childCount: 140 },
  { id: at("deviceLibrary", "node", "2"), name: "Sets", kind: "deviceFolder", depth: 3, expanded: false },
  { id: at("deviceLibrary", "node", "3"), name: "Techno", kind: "devicePlaylist", depth: 4, childCount: 49 },
  { id: at("oneLibrary", "library"), name: "OneLibrary", kind: "deviceLibrary", depth: 1 },
  { id: at("oneLibrary", "allTracks"), name: "All Tracks", kind: "deviceAllTracks", depth: 2, childCount: 140 },
  { id: at("oneLibrary", "playlists"), name: "Playlists", kind: "devicePlaylists", depth: 2 },
  { id: at("oneLibrary", "node", "1"), name: "Main", kind: "devicePlaylist", depth: 3, childCount: 140 },
];

/** A row as `kind depth name [count]`, for reading a whole tree at once. */
const line = (row: ShellTreeRow) =>
  `${row.kind} ${row.depth} ${row.name}${row.count === undefined ? "" : ` ${row.count}`}`;

describe("the shell's tree", () => {
  it("lists the Library's All Tracks, Intake and Problems, then every playlist at any depth, then Set plans", () => {
    expect(shellTreeRows({ nodes: LIBRARY }).map(line)).toEqual([
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
    const rows = shellTreeRows({ nodes: LIBRARY, collapsed: new Set(["f2"]) });
    expect(rows.map((row) => row.name)).not.toContain("Deeper");
    expect(rows.map((row) => row.name)).not.toContain("Bottom");
    // What follows the closed folder at its own level is still there.
    expect(rows.map((row) => row.name)).toContain("Fresh 128s");
    expect(rows.find((row) => row.id === "f2")).toMatchObject({ branch: true, open: false });
    expect(rows.find((row) => row.id === "f1")).toMatchObject({ branch: true, open: true });
  });

  it("opens Library rows on the Library's own nodes, and Intake and Problems on a queue", () => {
    const rows = shellTreeRows({ nodes: LIBRARY });
    expect(rows.find((row) => row.kind === "allTracks")?.node).toBe(LIBRARY[0]);
    expect(rows.find((row) => row.name === "Bottom")?.node).toBe(LIBRARY[6]);
    expect(rows.find((row) => row.kind === "intake")).toMatchObject({ queue: "intake" });
    expect(rows.find((row) => row.kind === "problems")).toMatchObject({ queue: "problems" });
    expect(rows.find((row) => row.kind === "intake")?.node).toBeUndefined();
  });

  it("lists Set plans once there are some", () => {
    const rows = shellTreeRows({ nodes: LIBRARY, setPlans: [{ id: "plan-1", name: "Friday peak hour", count: 16 }] });
    expect(rows.slice(-2).map(line)).toEqual(["heading 0 Set plans", "setPlan 0 Friday peak hour 16"]);
  });

  it("lists a mounted Stick with the playlists of its first library, once, and nothing for a Stick that is gone", () => {
    const rows = shellTreeRows({ nodes: [...LIBRARY, ...STICK_NODES] });
    expect(rows.slice(rows.findIndex((row) => row.kind === "stick")).map(line)).toEqual([
      "stick 0 GIGSTICK",
      "stickPlaylist 0 Main 140",
      "stickFolder 0 Sets",
      "stickPlaylist 1 Techno 49",
    ]);
    expect(rows.find((row) => row.kind === "stick")?.node).toBe(STICK_NODES[0]);
    expect(rows.find((row) => row.name === "Techno")?.node).toBe(STICK_NODES[6]);
    expect(shellTreeRows({ nodes: LIBRARY }).some((row) => row.kind === "stick")).toBe(false);
  });

  it("says so when a Stick that has been read holds no playlists, and says nothing before it is read", () => {
    const unread = shellTreeRows({ nodes: [STICK_NODES[0] as TreeNode] });
    expect(unread.slice(unread.findIndex((row) => row.kind === "stick")).map(line)).toEqual(["stick 0 GIGSTICK"]);
    const bare = shellTreeRows({ nodes: STICK_NODES.slice(0, 4) });
    expect(bare.slice(bare.findIndex((row) => row.kind === "stick")).map(line)).toEqual([
      "stick 0 GIGSTICK",
      "empty 0 No playlists",
    ]);
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
