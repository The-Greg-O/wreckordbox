import { describe, expect, it } from "vitest";
import type { TreeNode } from "@/ipc/types";
import { selectionInTree } from "./treeSelection";

const all: TreeNode = { id: "all", name: "All Tracks", kind: "allTracks", depth: 0 };
const vox: TreeNode = { id: "vox", name: "Melodic Vox", kind: "playlist", depth: 1 };
const made: TreeNode = { id: "made-1", name: "New playlist", kind: "playlist", depth: 1 };
const tree = [all, vox];

describe("selectionInTree", () => {
  it("keeps a node the tree still holds", () => {
    expect(selectionInTree(vox, tree)).toEqual({ node: vox, gone: null });
  });

  it("follows a node renamed in the tree", () => {
    const renamed = { ...vox, name: "Vox 2" };
    expect(selectionInTree(vox, [all, renamed])).toEqual({ node: renamed, gone: null });
  });

  it("leaves a playlist the Library no longer has for All Tracks, and names it", () => {
    expect(selectionInTree(made, tree)).toEqual({ node: all, gone: "New playlist" });
  });

  it("leaves a gone folder, history or intelligent playlist the same way", () => {
    for (const kind of ["folder", "history", "smartPlaylist"] as const) {
      expect(selectionInTree({ ...made, kind }, tree).node).toBe(all);
    }
  });

  it("does not touch a node the Library tree never holds (a Stick, the Tag List, a folder on disk)", () => {
    for (const kind of ["device", "tagList", "directory", "relatedCriterion"] as const) {
      const node = { ...made, kind };
      expect(selectionInTree(node, tree)).toEqual({ node, gone: null });
    }
  });

  it("falls back to the first node when there is no All Tracks", () => {
    expect(selectionInTree(made, [vox])).toEqual({ node: vox, gone: "New playlist" });
    expect(selectionInTree(made, [])).toEqual({ node: null, gone: "New playlist" });
  });

  it("keeps no selection as none", () => {
    expect(selectionInTree(null, tree)).toEqual({ node: null, gone: null });
  });
});
