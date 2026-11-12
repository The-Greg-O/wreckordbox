import { describe, expect, it } from "vitest";

import type { StickWritePlan } from "@/ipc/types";
import type { ShellStick } from "./shellTree";
import { describeStickPlan, stickHoldMs, stickKind, stickRefusal } from "./stickWrites";

const plan = (over: Partial<StickWritePlan> = {}): StickWritePlan => ({
  spare: false, pending: false, audioEstimated: true, tracksNew: 0, tracksChanged: 0, audioReused: 0, tracksRemoved: 0, tracksMissing: 0,
  trackChanges: [], analysisWritten: 0, playlists: [], netAdditionalBytes: 0,
  onStick: true, stickName: "USB", protected: false, adding: null, plan: "p", ...over,
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
  it("tells audio copied from audio reused, and says the reused tracks' data is still written", () => {
    expect(describeStickPlan(plan({
      tracksNew: 2, tracksChanged: 1, audioReused: 40, analysisWritten: 43,
      trackChanges: [{ title: "Ritual", fields: ["Energy", "MyTags"] }],
      playlists: [
        { name: "Warmup", folder: false, change: "resynchronised", added: 1, removed: 2, reordered: false },
        { name: "Peak", folder: false, change: "created", added: 5, removed: 0, reordered: false },
        { name: "Crate", folder: true, change: "created", added: null, removed: null, reordered: null },
        { name: "Old", folder: false, change: "resynchronised", added: 0, removed: 0, reordered: false },
        { name: "Moved", folder: false, change: "resynchronised", added: 0, removed: 0, reordered: true },
        { name: "Grown", folder: false, change: "resynchronised", added: 2, removed: 0, reordered: true },
        { name: "Unknown", folder: false, change: "resynchronised", added: null, removed: null, reordered: null },
      ],
      netAdditionalBytes: 16_000_000,
    }))).toEqual([
      "2 tracks' audio copied to the Stick",
      "1 track's audio copied again (changed at its source)",
      "40 tracks' audio already on the Stick, reused; their data is written again",
      "“Ritual”: Energy, MyTags changed",
      "Cues and analysis of 43 tracks written from the Library (changes to them are not listed)",
      "Playlist “Peak” created with 5 tracks",
      "Folder “Crate” created",
      "Playlist “Warmup” written again: 1 track added, 2 removed",
      "Playlist “Old” written again, its tracks unchanged",
      "Playlist “Moved” written again: the same tracks in another order",
      "Playlist “Grown” written again: 2 tracks added, 0 removed, and the tracks it keeps in another order",
      "Playlist “Unknown” written again with the tracks the Library holds (its entries on the Stick could not be compared)",
      "About 16 MB more used on the Stick",
    ]);
  });
  it("lists at most ten changed tracks, then counts the rest", () => {
    const many = Array.from({ length: 12 }, (_, i) => ({ title: `T${i}`, fields: ["Title"] }));
    const lines = describeStickPlan(plan({ trackChanges: many }));
    expect(lines).toHaveLength(11);
    expect(lines.at(-1)).toBe("…and 2 more tracks whose data changes");
  });
  it("says how many entries an add to a Stick playlist puts in, as the backend resolved them", () => {
    expect(describeStickPlan(plan({ adding: 3 }))).toEqual(["3 entries go into the playlist"]);
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
