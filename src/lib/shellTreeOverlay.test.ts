import { describe, expect, it } from "vitest";

import type { TreeNode } from "@/ipc/types";
import { breadcrumbOf, treeKeyAction, GO_TO_TREE } from "./shellTreeOverlay";

const node = (id: string, name: string, kind: TreeNode["kind"], depth: number) => ({ id, name, kind, depth }) as TreeNode;
const TREE: TreeNode[] = [
  node("1", "CURRENT", "folder", 0),
  node("2", "Melodic Vox", "playlist", 1),
  node("3", "Deep", "folder", 1),
  node("4", "Late", "playlist", 2),
  node("5", "Loose", "playlist", 0),
];

describe("breadcrumbOf", () => {
  it("names the folders above the open list, then the list", () => {
    expect(breadcrumbOf(TREE, { id: "2", name: "Melodic Vox" })).toEqual(["CURRENT", "Melodic Vox"]);
    expect(breadcrumbOf(TREE, { id: "4", name: "Late" })).toEqual(["CURRENT", "Deep", "Late"]);
    expect(breadcrumbOf(TREE, { id: "5", name: "Loose" })).toEqual(["Loose"]);
  });

  it("names what the tree does not hold (a Stick's playlist, All Tracks) by its own name", () => {
    expect(breadcrumbOf(TREE, { id: "device:x", name: "Main" })).toEqual(["Main"]);
    expect(breadcrumbOf(TREE, null)).toEqual(["Collection"]);
  });
});

describe("treeKeyAction", () => {
  const linux = { mac: false };
  const at = (tag: string) => ({ tagName: tag, isContentEditable: false, closest: () => null }) as unknown as HTMLElement;
  it("is Ctrl+T, not in a text field", () => {
    expect(GO_TO_TREE.chord).toEqual({ key: "t", metaKey: true });
    expect(treeKeyAction({ key: "t", code: "KeyT", ctrlKey: true, metaKey: false, shiftKey: false, altKey: false }, linux as never, at("DIV"))).toBe(true);
    expect(treeKeyAction({ key: "t", code: "KeyT", ctrlKey: true, metaKey: false, shiftKey: false, altKey: false }, linux as never, at("INPUT"))).toBe(false);
    expect(treeKeyAction({ key: "t", code: "KeyT", ctrlKey: false, metaKey: false, shiftKey: false, altKey: false }, linux as never, at("DIV"))).toBe(false);
  });
});
