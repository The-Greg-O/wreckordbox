import { describe, expect, it } from "vitest";

import type { TreeNode } from "@/ipc/types";
import {
  dropInOrder, formatTargetLength, moveInOrder, navigateTarget, sourceOwnsKeyboard, swapLists, targetCandidates, targetKeyAction, targetRefusal,
} from "./targetList";

const linux = { mac: false, windows: false, linux: true };

const node = (id: string, name: string, kind: TreeNode["kind"], depth = 1): TreeNode => ({ id, name, kind, depth });

const TREE: TreeNode[] = [
  node("collection", "Collection", "collection", 0),
  node("f-1", "GIGS", "folder", 0),
  node("pl-1", "Friday warm-up", "playlist", 1),
  node("smart-1", "Fresh 128s", "smartPlaylist", 1),
  node("pl-2", "Melodic Vox", "playlist", 0),
];

describe("formatTargetLength", () => {
  it("draws hours and minutes as the design does", () => {
    expect(formatTargetLength(78 * 60 + 20)).toBe("1h18m");
    expect(formatTargetLength(2 * 3600)).toBe("2h00m");
  });
  it("draws minutes alone under an hour, and an empty list as nothing long", () => {
    expect(formatTargetLength(42 * 60 + 59)).toBe("42m");
    expect(formatTargetLength(0)).toBe("0m");
  });
});

describe("targetCandidates", () => {
  it("offers every plain Library playlist, named with its folders", () => {
    expect(targetCandidates(TREE)).toEqual([
      { id: "pl-1", name: "Friday warm-up", path: "GIGS / Friday warm-up" },
      { id: "pl-2", name: "Melodic Vox", path: "Melodic Vox" },
    ]);
  });
});

describe("targetRefusal", () => {
  it("takes a plain Library playlist", () => {
    expect(targetRefusal(node("pl-1", "Friday warm-up", "playlist"))).toBeNull();
  });
  it("refuses a Stick's playlist, saying why", () => {
    expect(targetRefusal(node("dev:x", "Main", "devicePlaylist"))).toMatch(/Stick/);
  });
  it("refuses an intelligent playlist, a folder and All Tracks", () => {
    expect(targetRefusal(node("smart-1", "Fresh 128s", "smartPlaylist"))).toMatch(/intelligent/i);
    expect(targetRefusal(node("f-1", "GIGS", "folder"))).toMatch(/folder/i);
    expect(targetRefusal(node("collection", "Collection", "collection"))).toMatch(/playlist/i);
    expect(targetRefusal(null)).toMatch(/playlist/i);
  });
});

describe("moveInOrder", () => {
  it("moves one row up or down and follows it", () => {
    expect(moveInOrder(["a", "b", "c"], 1, -1)).toEqual({ order: ["b", "a", "c"], index: 0 });
    expect(moveInOrder(["a", "b", "c"], 1, 1)).toEqual({ order: ["a", "c", "b"], index: 2 });
  });
  it("does nothing past either end", () => {
    expect(moveInOrder(["a", "b"], 0, -1)).toBeNull();
    expect(moveInOrder(["a", "b"], 1, 1)).toBeNull();
    expect(moveInOrder([], 0, 1)).toBeNull();
  });
});

describe("dropInOrder", () => {
  it("puts a dragged row before the row it is dropped on", () => {
    expect(dropInOrder(["a", "b", "c", "d"], 3, 1)).toEqual(["a", "d", "b", "c"]);
    expect(dropInOrder(["a", "b", "c", "d"], 0, 2)).toEqual(["b", "a", "c", "d"]);
  });
  it("puts it last when dropped below every row", () => {
    expect(dropInOrder(["a", "b", "c"], 0, 3)).toEqual(["b", "c", "a"]);
  });
  it("is nothing when dropped where it already is", () => {
    expect(dropInOrder(["a", "b", "c"], 1, 1)).toBeNull();
    expect(dropInOrder(["a", "b", "c"], 1, 2)).toBeNull();
  });
});

describe("swapLists", () => {
  const friday = node("pl-1", "Friday warm-up", "playlist");
  const vox = node("pl-2", "Melodic Vox", "playlist");
  it("opens the target in the main list and makes the source the target", () => {
    expect(swapLists(vox, friday)).toEqual({ open: friday, target: vox });
  });
  it("refuses when either side cannot be the other's, saying why", () => {
    expect(swapLists(node("collection", "Collection", "collection"), friday)).toEqual({ refusal: expect.stringMatching(/All Tracks|playlist/) });
    expect(swapLists(node("dev:x", "Main", "devicePlaylist"), friday)).toEqual({ refusal: expect.stringMatching(/Stick/) });
    expect(swapLists(vox, null)).toEqual({ refusal: expect.stringMatching(/target/i) });
  });
});

describe("targetKeyAction", () => {
  const key = (init: Partial<KeyboardEventInit> & { key: string }) => ({ code: "", ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...init });
  it("reads the second list's keys", () => {
    expect(targetKeyAction(key({ key: "\\", code: "Backslash", ctrlKey: true }), linux)).toBe("toggleTargetList");
    expect(targetKeyAction(key({ key: "|", code: "Backslash", ctrlKey: true, shiftKey: true }), linux)).toBe("swapTargetLists");
    expect(targetKeyAction(key({ key: "Enter" }), linux)).toBe("addToTarget");
    expect(targetKeyAction(key({ key: "ArrowUp", altKey: true }), linux)).toBe("moveTargetUp");
    expect(targetKeyAction(key({ key: "ArrowDown", altKey: true }), linux)).toBe("moveTargetDown");
  });
  it("leaves every other key alone", () => {
    expect(targetKeyAction(key({ key: "Enter", shiftKey: true }), linux)).toBeNull();
    expect(targetKeyAction(key({ key: "ArrowUp" }), linux)).toBeNull();
    expect(targetKeyAction(key({ key: "\\", code: "Backslash", ctrlKey: true, altKey: true }), linux)).toBeNull();
  });
  it("leaves keys to a field, a dialog, and a control that owns its Enter", () => {
    const inside = (selector: string) => ({ tagName: "DIV", isContentEditable: false, closest: (s: string) => (s.includes(selector) ? {} : null) });
    const field = { tagName: "INPUT", isContentEditable: false, closest: () => null };
    expect(targetKeyAction(key({ key: "Enter" }), linux, field)).toBeNull();
    expect(targetKeyAction(key({ key: "\\", code: "Backslash", ctrlKey: true }), linux, field)).toBeNull();
    expect(targetKeyAction(key({ key: "Enter" }), linux, inside('[role="dialog"]'))).toBeNull();
    expect(targetKeyAction(key({ key: "Enter" }), linux, inside("[data-own-activation]"))).toBeNull();
    // Only Enter is a control's own: the second list's other keys still reach it.
    expect(targetKeyAction(key({ key: "ArrowUp", altKey: true }), linux, inside("[data-own-activation]"))).toBe("moveTargetUp");
  });
});

describe("sourceOwnsKeyboard", () => {
  const body = { tagName: "BODY", closest: () => null };
  const inList = { tagName: "DIV", closest: (s: string) => (s.includes('data-browser-pane="list"') ? {} : null) };
  const treeItem = { tagName: "DIV", closest: () => null };
  const mainList = { pane: "list", side: "main" } as const;
  it("is the main list's after a press in it, with the page or the list holding the keyboard", () => {
    expect(sourceOwnsKeyboard(body, mainList)).toBe(true);
    expect(sourceOwnsKeyboard(inList, mainList)).toBe(true);
    expect(sourceOwnsKeyboard(null, mainList)).toBe(true);
  });
  it("is not when the tree, the second list or a control holds the keyboard", () => {
    expect(sourceOwnsKeyboard(treeItem, mainList)).toBe(false);
    expect(sourceOwnsKeyboard(body, { pane: "tree", side: "main" })).toBe(false);
    expect(sourceOwnsKeyboard(body, null)).toBe(false);
    expect(sourceOwnsKeyboard(body, { pane: "list", side: "sub" })).toBe(false);
  });
});

describe("navigateTarget", () => {
  it("moves the cursor by the list's own commands, staying inside the list", () => {
    expect(navigateTarget("moveDown", null, 5, 3)).toBe(0);
    expect(navigateTarget("moveDown", 1, 5, 3)).toBe(2);
    expect(navigateTarget("moveDown", 4, 5, 3)).toBe(4);
    expect(navigateTarget("moveUp", 0, 5, 3)).toBe(0);
    expect(navigateTarget("moveUp", null, 5, 3)).toBe(4);
    expect(navigateTarget("pageDown", 1, 10, 3)).toBe(4);
    expect(navigateTarget("pageDown", 8, 10, 3)).toBe(9);
    expect(navigateTarget("pageUp", 2, 10, 3)).toBe(0);
    expect(navigateTarget("toTop", 6, 10, 3)).toBe(0);
    expect(navigateTarget("toBottom", 0, 10, 3)).toBe(9);
    expect(navigateTarget("extendDown", 1, 5, 3)).toBe(2);
  });
  it("is nothing in an empty list, and Select All keeps the one row", () => {
    expect(navigateTarget("moveDown", null, 0, 3)).toBeNull();
    expect(navigateTarget("selectAll", 2, 5, 3)).toBe(2);
  });
  it("is not the list's for any other command", () => {
    expect(navigateTarget("playPause", 1, 5, 3)).toBeUndefined();
  });
});
