import { describe, expect, it } from "vitest";

import type { StickWritePlan } from "@/ipc/types";
import type { ShellStick } from "./shellTree";
import { describeStickPlan, stickHoldMs, stickKind, stickRefusal } from "./stickWrites";

const plan = (over: Partial<StickWritePlan> = {}): StickWritePlan => ({
  spare: false, pending: false, tracksNew: 0, tracksChanged: 0, tracksKept: 0, tracksRemoved: 0, tracksMissing: 0,
  playlists: [], netAdditionalBytes: 0, ...over,
});
const stick = (outline: ShellStick["outline"]): ShellStick => ({ path: "/media/USB", name: "USB", outline });

describe("the hold a Stick write asks for", () => {
  it("is twice as long for a Gig stick as for a Spare stick, as a load's is", () => {
    expect(stickHoldMs(true)).toBe(1000);
    expect(stickHoldMs(false)).toBe(2000);
    expect(stickKind(true)).toBe("Spare stick");
    expect(stickKind(false)).toBe("Gig stick");
  });
});

describe("stickRefusal", () => {
  it("lets a read, settled Stick be written to", () => {
    expect(stickRefusal(stick({ pending: false, libraries: [] }), false)).toBeNull();
  });
  it("refuses a Stick not read yet, one whose last write did not finish, one being checked after a write, and one gone", () => {
    expect(stickRefusal(stick(null), false)).toMatch(/not been read yet/);
    expect(stickRefusal(stick({ pending: true, libraries: [] }), false)).toMatch(/did not finish.*nothing was written/s);
    expect(stickRefusal(stick({ pending: false, libraries: [] }), true)).toMatch(/being checked/);
    expect(stickRefusal(undefined, false)).toMatch(/no longer connected/);
  });
});

describe("describeStickPlan", () => {
  it("says every track and playlist the write changes", () => {
    expect(describeStickPlan(plan({
      tracksNew: 2, tracksChanged: 1, tracksRemoved: 0, tracksKept: 40,
      playlists: [
        { name: "Warmup", folder: false, change: "resynchronised" },
        { name: "Peak", folder: false, change: "created" },
        { name: "Crate", folder: true, change: "created" },
      ],
      netAdditionalBytes: 16_000_000,
    }))).toEqual([
      "2 tracks copied to the Stick",
      "1 track on the Stick rewritten",
      "40 tracks already there left as they are",
      "Playlist “Peak” created",
      "Folder “Crate” created",
      "Playlist “Warmup” written again from the Library",
      "About 16 MB more used on the Stick",
    ]);
  });
  it("names removals and missing files, and a write that frees space", () => {
    expect(describeStickPlan(plan({ tracksRemoved: 3, tracksMissing: 1, netAdditionalBytes: -5_000_000 }))).toEqual([
      "3 tracks removed from the Stick",
      "1 track not written: its file is missing",
      "About 5 MB freed on the Stick",
    ]);
  });
});

describe("shellOpensNode", () => {
  it("opens a Stick playlist only while its Stick may be used", async () => {
    const { shellOpensNode } = await import("./stickWrites");
    const { deviceNodeId } = await import("./deviceLibrary");
    const node = { id: deviceNodeId({ path: "/media/USB", format: "deviceLibrary", role: "node", id: "3" }), name: "Set", kind: "devicePlaylist" as const, depth: 3 };
    expect(shellOpensNode(node, () => null)).toBe(node);
    expect(shellOpensNode(node, () => "A write to USB did not finish.")).toBeNull();
    const playlist = { id: "p1", name: "Warmup", kind: "playlist" as const, depth: 1 };
    expect(shellOpensNode(playlist, () => "refused")).toBe(playlist);
  });
});
