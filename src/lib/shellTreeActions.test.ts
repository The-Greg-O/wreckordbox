import { describe, expect, it } from "vitest";

import type { StickOutline, TreeNode } from "@/ipc/types";
import { deviceNodeId } from "./deviceLibrary";
import { shellTreeRows, type ShellStick, type ShellTreeRow } from "./shellTree";
import {
  moveDestination, playlistsMenu, setShellFolderOpen, shellCollapsed, shellRowDrop, shellRowMenu,
  shellRowMovable, shellRowRenamable,
} from "./shellTreeActions";

const library: TreeNode[] = [
  { id: "all", name: "All Tracks", kind: "allTracks", depth: 0, childCount: 10 },
  { id: "playlists", name: "Playlists", kind: "collection", depth: 0 },
  { id: "f1", name: "Crate", kind: "folder", depth: 1 },
  { id: "p1", name: "Warmup", kind: "playlist", depth: 2, childCount: 3 },
  { id: "p2", name: "Peak", kind: "playlist", depth: 2, childCount: 4 },
  { id: "s1", name: "Fresh", kind: "smartPlaylist", depth: 1 },
  { id: "p3", name: "Closing", kind: "playlist", depth: 1 },
];

const outline = (pending = false): StickOutline => ({
  pending,
  libraries: pending ? [] : [
    { format: "deviceLibrary", nodes: [{ id: "10", name: "Gig", folder: true, depth: 0 }, { id: "11", name: "Set", folder: false, depth: 1, count: 2 }] },
    { format: "oneLibrary", nodes: [{ id: "10", name: "Gig", folder: true, depth: 0 }, { id: "11", name: "Set", folder: false, depth: 1, count: 2 }] },
  ],
});

const stick = (pending = false): ShellStick => ({ path: "/media/USB", name: "USB", outline: outline(pending) });

function rowsOf(sticks: ShellStick[] = [stick()]): ShellTreeRow[] {
  return shellTreeRows({ library, sticks });
}
function row(rows: readonly ShellTreeRow[], name: string, kind?: ShellTreeRow["kind"]): ShellTreeRow {
  const found = rows.find((r) => r.name === name && (kind === undefined || r.kind === kind));
  if (!found) throw new Error(`no row ${name}`);
  return found;
}

describe("shellRowMenu", () => {
  const rows = rowsOf();
  it("gives each Library list its old tree menu, on its own node", () => {
    expect(shellRowMenu(row(rows, "Warmup"), [stick()])).toMatchObject({ kind: "tree", menu: "playlist", node: { id: "p1" } });
    expect(shellRowMenu(row(rows, "Fresh"), [stick()])).toMatchObject({ kind: "tree", menu: "smartPlaylist", node: { id: "s1" } });
    expect(shellRowMenu(row(rows, "Crate"), [stick()])).toMatchObject({ kind: "tree", menu: "folder", node: { id: "f1" } });
  });

  it("has nothing for All Tracks, the queues, headings and empty lines", () => {
    for (const r of rows.filter((r) => ["allTracks", "intake", "problems", "heading", "empty"].includes(r.kind))) {
      expect(shellRowMenu(r, [stick()])).toBeNull();
    }
  });

  it("gives a Stick's playlists and folders the Devices tree's menus", () => {
    expect(shellRowMenu(row(rows, "Set"), [stick()])).toMatchObject({ kind: "stick", menu: "devicePlaylist" });
    expect(shellRowMenu(row(rows, "Gig"), [stick()])).toMatchObject({ kind: "stick", menu: "deviceFolder" });
  });

  it("gives the Stick row its first library's Playlists heading menu, and Eject", () => {
    const menu = shellRowMenu(row(rows, "USB", "stick"), [stick()]);
    expect(menu).toMatchObject({
      kind: "stick",
      menu: "devicePlaylists",
      node: { kind: "devicePlaylists", id: deviceNodeId({ path: "/media/USB", format: "deviceLibrary", role: "playlists", id: "0" }) },
      eject: { kind: "device", id: "device:/media/USB" },
    });
  });

  it("offers only Eject on a Stick whose last write did not finish, or not read yet", () => {
    const pending = rowsOf([stick(true)]);
    expect(shellRowMenu(row(pending, "USB", "stick"), [stick(true)])).toEqual({
      kind: "eject", eject: { id: "device:/media/USB", name: "USB", kind: "device", depth: 0 },
    });
    const unread: ShellStick = { path: "/media/USB", name: "USB", outline: null };
    const rowsUnread = shellTreeRows({ library, sticks: [unread] });
    expect(shellRowMenu(row(rowsUnread, "USB", "stick"), [unread])).toMatchObject({ kind: "eject" });
  });

  it("gives each library's label its own Playlists heading menu when the two differ", () => {
    const split: ShellStick = {
      path: "/media/USB", name: "USB",
      outline: { pending: false, libraries: [
        { format: "deviceLibrary", nodes: [{ id: "1", name: "A", folder: false, depth: 0 }] },
        { format: "oneLibrary", nodes: [] },
      ] },
    };
    const rows = shellTreeRows({ library, sticks: [split] });
    expect(shellRowMenu(row(rows, "OneLibrary"), [split])).toMatchObject({
      kind: "stick", menu: "devicePlaylists",
      node: { id: deviceNodeId({ path: "/media/USB", format: "oneLibrary", role: "playlists", id: "0" }) },
    });
    // The Stick row itself cannot say which library a new playlist is for.
    expect(shellRowMenu(row(rows, "USB", "stick"), [split])).toMatchObject({ kind: "eject" });
  });
});

describe("playlistsMenu", () => {
  it("is the old tree's Playlists heading menu, on the collection node", () => {
    expect(playlistsMenu(library)).toMatchObject({ kind: "tree", menu: "collection", node: { id: "playlists" } });
    expect(playlistsMenu([])).toBeNull();
  });
});

describe("shellRowDrop", () => {
  const rows = rowsOf();
  const tracks = { tracks: true, toStick: true, node: false, files: false };
  it("takes tracks on a Library playlist, not on a folder or an intelligent playlist", () => {
    expect(shellRowDrop(row(rows, "Warmup"), [stick()], tracks)).toBe("tracks");
    expect(shellRowDrop(row(rows, "Crate"), [stick()], tracks)).toBeNull();
    expect(shellRowDrop(row(rows, "Fresh"), [stick()], tracks)).toBeNull();
  });
  it("takes tracks on a Stick playlist and exports them to a Stick", () => {
    expect(shellRowDrop(row(rows, "Set"), [stick()], tracks)).toBe("stickTracks");
    expect(shellRowDrop(row(rows, "USB", "stick"), [stick()], tracks)).toBe("exportTracks");
  });
  it("exports no loose files to a Stick, and nothing to a Stick whose last write did not finish", () => {
    expect(shellRowDrop(row(rows, "USB", "stick"), [stick()], { ...tracks, toStick: false })).toBeNull();
    const pending = rowsOf([stick(true)]);
    expect(shellRowDrop(row(pending, "USB", "stick"), [stick(true)], tracks)).toBeNull();
    expect(shellRowDrop(row(pending, "USB", "stick"), [stick(true)], { tracks: false, toStick: false, node: true, files: false })).toBeNull();
  });
  it("exports a playlist or folder dropped on a Stick", () => {
    expect(shellRowDrop(row(rows, "USB", "stick"), [stick()], { tracks: false, toStick: false, node: true, files: false })).toBe("exportNode");
  });
  it("takes files on All Tracks, a playlist and a folder", () => {
    const files = { tracks: false, toStick: false, node: false, files: true };
    expect(shellRowDrop(row(rows, "All Tracks"), [stick()], files)).toBe("files");
    expect(shellRowDrop(row(rows, "Warmup"), [stick()], files)).toBe("files");
    expect(shellRowDrop(row(rows, "Crate"), [stick()], files)).toBe("files");
    expect(shellRowDrop(row(rows, "Intake"), [stick()], files)).toBeNull();
    expect(shellRowDrop(row(rows, "Set"), [stick()], files)).toBeNull();
  });
});

describe("what can be renamed and moved", () => {
  const rows = rowsOf();
  it("renames Library lists and a Stick's own lists", () => {
    expect(["Warmup", "Fresh", "Crate", "Set", "Gig"].map((n) => shellRowRenamable(row(rows, n)))).toEqual([true, true, true, true, true]);
    expect(shellRowRenamable(row(rows, "All Tracks"))).toBe(false);
    expect(shellRowRenamable(row(rows, "USB", "stick"))).toBe(false);
  });
  it("moves only the Library's lists", () => {
    expect(["Warmup", "Fresh", "Crate"].map((n) => shellRowMovable(row(rows, n)))).toEqual([true, true, true]);
    expect(shellRowMovable(row(rows, "Set"))).toBe(false);
    expect(shellRowMovable(row(rows, "All Tracks"))).toBe(false);
  });
});

describe("moveDestination", () => {
  const node = (id: string) => library.find((n) => n.id === id) as TreeNode;
  it("appends into a folder's middle", () => {
    expect(moveDestination(library, node("p3"), node("f1"), "into")).toEqual({ parent: "f1", index: 2 });
  });
  it("places beside a sibling, counted with the moved node lifted out", () => {
    expect(moveDestination(library, node("p3"), node("f1"), "above")).toEqual({ parent: "root", index: 0 });
    expect(moveDestination(library, node("p1"), node("p2"), "below")).toEqual({ parent: "f1", index: 1 });
    expect(moveDestination(library, node("p3"), node("p1"), "above")).toEqual({ parent: "f1", index: 0 });
  });
  it("lands only beside or in a playlist or folder, as the old tree's moves do", () => {
    expect(moveDestination(library, node("p3"), node("s1"), "above")).toBeNull();
  });
  it("refuses a node's own subtree", () => {
    expect(moveDestination(library, node("f1"), node("p1"), "above")).toBeNull();
    expect(moveDestination(library, node("f1"), node("f1"), "into")).toBeNull();
  });
});

describe("folder open and closed, kept in the session's treeExpansion", () => {
  it("closing adds to collapsed and drops from expanded; opening the reverse", () => {
    const start = { collapsed: ["x"], expanded: ["f1"] };
    const closed = setShellFolderOpen(start, "f1", false);
    expect(closed).toEqual({ collapsed: ["x", "f1"], expanded: [] });
    expect(shellCollapsed(closed)).toEqual(new Set(["x", "f1"]));
    expect(setShellFolderOpen(closed, "f1", true)).toEqual({ collapsed: ["x"], expanded: ["f1"] });
  });
  it("returns the same value when nothing changes", () => {
    const start = { collapsed: ["f1"], expanded: [] };
    expect(setShellFolderOpen(start, "f1", false)).toBe(start);
  });
});
