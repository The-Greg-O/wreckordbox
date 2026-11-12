import { describe, expect, it } from "vitest";

import type { TreeNode } from "@/ipc/types";
import { withDeviceLibraries } from "./deviceLibrary";
import { nodesForSource } from "./tree";
import { isUnfinishedWrite, UNFINISHED_WRITE, unfinishedWriteWording } from "./stickRecovery";

const stick: TreeNode = { id: "device:/Volumes/GIG", name: "GIG", kind: "device", depth: 0 };

describe("a Stick whose last write did not finish", () => {
  it("is told apart from any other failed read by the error's kind", () => {
    expect(isUnfinishedWrite({ kind: "unfinishedWrite", message: "x" })).toBe(true);
    expect(isUnfinishedWrite({ kind: "notFound", message: "x" })).toBe(false);
    expect(isUnfinishedWrite(new Error("unfinishedWrite"))).toBe(false);
    expect(isUnfinishedWrite(null)).toBe(false);
  });

  it("shows a line saying so under the Stick in the Devices tree, and nothing that opens", () => {
    const nodes = withDeviceLibraries([stick], new Map([["/Volumes/GIG", []]]), new Set(["/Volumes/GIG"]));
    expect(nodes.map((n) => [n.name, n.kind, n.depth])).toEqual([
      ["GIG", "device", 0],
      [UNFINISHED_WRITE, "note", 1],
    ]);
  });

  it("lists that line in the Devices section and not in the Explorer's", () => {
    const nodes = withDeviceLibraries([stick], new Map([["/Volumes/GIG", []]]), new Set(["/Volumes/GIG"]));
    const explorerNote: TreeNode = { id: "note:more", name: "12 more", kind: "note", depth: 1 };
    const all = [...nodes, explorerNote];
    expect(nodesForSource(all, "devices").map((n) => n.name)).toEqual(["GIG", UNFINISHED_WRITE]);
    expect(nodesForSource(all, "explorer").map((n) => n.name)).toEqual(["12 more"]);
  });

  it("names the Stick in the action and says what settling does", () => {
    const wording = unfinishedWriteWording("GIG");
    expect(wording.label).toBe("Unfinished write on GIG");
    expect(wording.action).toBe("Finish or roll back the unfinished write on GIG");
    expect(wording.what).toMatch(/finishes that export/);
    expect(wording.what).toMatch(/puts the Stick back/);
  });
});
