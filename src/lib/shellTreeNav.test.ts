import { describe, expect, it } from "vitest";

import type { TreeNode } from "@/ipc/types";
import { shellTreeRows } from "./shellTree";
import { keyTarget, treeLayout, visibleRange } from "./shellTreeNav";

const LIBRARY: TreeNode[] = [
  { id: "all", name: "All Tracks", kind: "allTracks", depth: 0 },
  { id: "f1", name: "CURRENT", kind: "folder", depth: 1 },
  { id: "p1", name: "Melodic Vox", kind: "playlist", depth: 2 },
  { id: "f2", name: "Deep", kind: "folder", depth: 2 },
  { id: "p2", name: "Bottom", kind: "playlist", depth: 3 },
  { id: "p3", name: "Loose", kind: "playlist", depth: 1 },
];
const STICK = { path: "/s", name: "S", outline: { pending: false, libraries: [{ format: "deviceLibrary" as const, nodes: [
  { id: "1", parentId: "0", name: "Main", folder: false, depth: 0, count: 1 },
] }] } };
const rows = shellTreeRows({ library: LIBRARY, sticks: [STICK] });
const at = (name: string) => rows.find((row) => row.name === name)?.id ?? "";
const go = (key: string, from: string) => keyTarget(rows, from, key);

describe("the shell tree's keys", () => {
  it("Up and Down move between places, skipping headings and empty lines, and stop at the ends", () => {
    expect(go("ArrowDown", at("All Tracks"))).toEqual({ focus: at("Intake") });
    expect(go("ArrowDown", at("Problems"))).toEqual({ focus: "f1" });
    expect(go("ArrowUp", "f1")).toEqual({ focus: at("Problems") });
    expect(go("ArrowUp", at("All Tracks"))).toEqual({ focus: at("All Tracks") });
    expect(go("ArrowDown", at("Main"))).toEqual({ focus: at("Main") });
  });

  it("Home and End go to the first and last place", () => {
    expect(go("Home", "p2")).toEqual({ focus: at("All Tracks") });
    expect(go("End", "p2")).toEqual({ focus: at("Main") });
  });

  it("Left closes an open folder, and otherwise goes to the parent; a Stick's top level goes to the Stick", () => {
    expect(go("ArrowLeft", "f2")).toEqual({ toggle: "f2", open: false });
    expect(go("ArrowLeft", "p2")).toEqual({ focus: "f2" });
    expect(go("ArrowLeft", "p1")).toEqual({ focus: "f1" });
    expect(go("ArrowLeft", at("Main"))).toEqual({ focus: "device:/s" });
    // At the top of a group there is nowhere to go: the key is still the tree's.
    expect(go("ArrowLeft", "p3")).toEqual({});
    const closed = shellTreeRows({ library: LIBRARY, collapsed: new Set(["f2"]) });
    expect(keyTarget(closed, "f2", "ArrowLeft")).toEqual({ focus: "f1" });
  });

  it("Right opens a closed folder, goes into an open one, and does nothing on a list", () => {
    const closed = shellTreeRows({ library: LIBRARY, collapsed: new Set(["f2"]) });
    expect(keyTarget(closed, "f2", "ArrowRight")).toEqual({ toggle: "f2", open: true });
    expect(go("ArrowRight", "f2")).toEqual({ focus: "p2" });
    expect(go("ArrowRight", "p1")).toEqual({});
    expect(go("ArrowRight", "device:/s")).toEqual({ focus: at("Main") });
  });

  it("lands on the Playlists heading and a Stick library's label when they are stops, as with actions (wbx-45.4)", () => {
    const stops = (row: { kind: string; id: string }) =>
      row.kind !== "heading" && row.kind !== "empty" || row.id === "shell:heading:playlists";
    expect(keyTarget(rows, "f1", "ArrowUp", stops as never)).toEqual({ focus: "shell:heading:playlists" });
    expect(keyTarget(rows, "shell:heading:playlists", "ArrowDown", stops as never)).toEqual({ focus: "f1" });
    expect(keyTarget(rows, "shell:heading:playlists", "ArrowUp", stops as never)).toEqual({ focus: at("Problems") });
  });
  it("leaves every other key alone", () => {
    expect(go("a", "p1")).toBeNull();
    expect(go("Enter", "p1")).toBeNull();
  });
});

describe("the shell tree's window", () => {
  const sizes = { row: 24, heading: 22, gap: 10, pad: 8 };

  it("lays groups out top to bottom: a heading, its rows, and a gap before the next", () => {
    const layout = treeLayout([[true, 2], [true, 1], [false, 3]], sizes);
    expect(layout.groups.map((g) => [g.top, g.rowsTop, g.height])).toEqual([
      [8, 30, 22 + 48],
      [8 + 70 + 10, 8 + 70 + 10 + 22, 22 + 24],
      [8 + 70 + 10 + 46 + 10, 8 + 70 + 10 + 46 + 10, 72],
    ]);
    expect(layout.height).toBe(8 + 70 + 10 + 46 + 10 + 72 + 8);
  });

  it("draws only the rows in view, with a few either side", () => {
    // (top of the group's rows, how many, row height, scroll, view height, spare)
    expect(visibleRange(0, 100_000, 24, 0, 240, 5)).toEqual([0, 15]);
    expect(visibleRange(0, 100_000, 24, 2400, 240, 5)).toEqual([95, 115]);
    expect(visibleRange(0, 100_000, 24, 2_399_760, 240, 5)).toEqual([99_985, 100_000]);
    // A group below the view draws none of its rows.
    expect(visibleRange(1000, 50, 24, 0, 240, 5)).toEqual([0, 0]);
  });
});
