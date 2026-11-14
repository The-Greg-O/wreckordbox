import { describe, expect, it } from "vitest";

import {
  chosenAfterToggle,
  outcomeLines,
  formatBytes,
  fromServiceDryRun,
  fromServiceLoad,
  fromServiceSticks,
  lastLoadedLine,
  loadBlocked,
  shortfallLine,
  spaceLine,
  summaryLines,
  type LoadPlan,
} from "./stickLoad";

const serviceStick = (over: Record<string, unknown> = {}) => ({
  path: "/media/GIGSTICK", name: "GIGSTICK", kind: "gig", protected: true, unfinished_write: false,
  last_loaded: "2026-11-07 21:04:00.000 +00:00", free_bytes: 40_000_000_000, total_bytes: null, tracks: 1204, playlists: 38,
  ...over,
});

const serviceDryRun = (over: Record<string, unknown> = {}) => ({
  stick: serviceStick(),
  playlists: [{ id: "11", name: "Friday warm-up" }, { id: "12", name: "Rooftop closing" }, { id: "13", name: "Melodic Vox" }],
  changes: {
    counts: {
      tracks_added: 64, tracks_changed: 3, audio_changed: 1, tracks_removed: 6, tracks_missing: 0, audio_reused: 900,
      playlists_created: 2, playlists_rewritten: 7, playlists_reordered: 5, playlists_removed: 1, native_stems: 12,
    },
    tracks: {
      added: [{ id: "1", title: "Alpha" }],
      changed: [
        { id: "2", title: "Beta", fields: ["MyTags", "Energy"] },
        { id: "3", title: "Gamma", fields: ["MyTags"] },
        { id: "4", title: "Delta", fields: ["Energy", "BPM"] },
      ],
      audio_changed: [{ id: "5", title: "Echo" }],
      removed: [{ id: "6", title: "Foxtrot" }],
      missing: [],
    },
    playlists: { created: ["Friday warm-up", "Rooftop closing"], rewritten: [], reordered: ["Melodic Vox"], removed: ["Old set"] },
    native_stems: [{ id: "7", title: "Golf" }],
    analysis_written: 70,
    audio_estimated: true,
  },
  space: { free_before: 40_000_000_000, needed_bytes: 3_000_000_000, free_after: 37_000_000_000, shortfall_bytes: null, native_stems_bytes: 0, approximate: true },
  plan: "plan-1",
  not_loadable: null,
  ...over,
});

const plan = (over: Record<string, unknown> = {}): LoadPlan => fromServiceDryRun(serviceDryRun(over));

describe("fromServiceSticks", () => {
  it("reads the service's Sticks in the view's terms", () => {
    expect(fromServiceSticks({ sticks: [serviceStick({ kind: "spare", protected: false })] })).toEqual([{
      path: "/media/GIGSTICK", name: "GIGSTICK", kind: "spare", protected: false, unfinishedWrite: false,
      lastLoaded: "2026-11-07 21:04:00.000 +00:00", freeBytes: 40_000_000_000, totalBytes: null, tracks: 1204, playlists: 38,
    }]);
  });

  it("takes anything but the Spare stick marker for a Gig stick", () => {
    expect(fromServiceSticks({ sticks: [serviceStick({ kind: "odd" })] })[0]?.kind).toBe("gig");
  });
});

describe("summaryLines", () => {
  it("says what the load will do by kind of change, in the DJ's words", () => {
    expect(summaryLines(plan()).map((line) => [line.kind, line.count, line.text])).toEqual([
      ["added", "+64", "tracks added, across Friday warm-up, Rooftop closing and Melodic Vox"],
      ["stems", "12", "tracks get Native stems for the XDJ-AZ pads"],
      ["changed", "3", "tracks updated · MyTags 2 · Energy 2 · BPM 1"],
      ["audio", "1", "track's audio copied again: it changed at the source"],
      ["playlists", "2", "playlists created · 5 reordered"],
      ["removed", "-6", "tracks removed"],
      ["playlistsRemoved", "1", "playlist taken off the Stick: Old set"],
    ]);
  });

  it("names tracks left behind because their audio is missing", () => {
    const missing = summaryLines(plan({ changes: { ...serviceDryRun().changes, counts: { ...serviceDryRun().changes.counts, tracks_missing: 2 } } }));
    expect(missing.find((line) => line.kind === "missing")?.text).toBe("tracks not loaded: their audio file is missing");
  });

  it("says plainly when there is nothing to do", () => {
    const none = {
      counts: {
        tracks_added: 0, tracks_changed: 0, audio_changed: 0, tracks_removed: 0, tracks_missing: 0, audio_reused: 4,
        playlists_created: 0, playlists_rewritten: 0, playlists_reordered: 0, playlists_removed: 0, native_stems: 0,
      },
      tracks: { added: [], changed: [], audio_changed: [], removed: [], missing: [] },
      playlists: { created: [], rewritten: [], reordered: [], removed: [] },
      native_stems: [], analysis_written: 4, audio_estimated: true,
    };
    expect(summaryLines(plan({ changes: none })).map((line) => line.text))
      .toEqual(["Nothing to change: the Stick already holds these playlists as they are."]);
  });

  it("counts playlists whose tracks change even when no track is added or removed from the Stick", () => {
    const base = serviceDryRun().changes;
    const none = {
      ...base,
      counts: {
        tracks_added: 0, tracks_changed: 0, audio_changed: 0, tracks_removed: 0, tracks_missing: 0, audio_reused: 4,
        playlists_created: 0, playlists_rewritten: 2, playlists_reordered: 0, playlists_removed: 0, native_stems: 0,
      },
      tracks: { added: [], changed: [], audio_changed: [], removed: [], missing: [] },
      playlists: {
        created: [],
        rewritten: [
          { name: "Friday warm-up", added: 1, removed: 0, reordered: false },
          { name: "Melodic Vox", added: 0, removed: 0, reordered: false },
        ],
        reordered: [], removed: [],
      },
      native_stems: [],
    };
    expect(summaryLines(plan({ changes: none })).map((line) => [line.kind, line.count, line.text])).toEqual([
      ["membership", "1", "playlist's tracks change: Friday warm-up (+1)"],
    ]);
  });

  it("says nothing changes only when no playlist's tracks change either", () => {
    const base = serviceDryRun().changes;
    const same = {
      ...base,
      counts: { ...base.counts, tracks_added: 0, tracks_changed: 0, audio_changed: 0, tracks_removed: 0, playlists_created: 0, playlists_reordered: 0, playlists_removed: 0, native_stems: 0 },
      tracks: { added: [], changed: [], audio_changed: [], removed: [], missing: [] },
      playlists: { created: [], rewritten: [{ name: "Melodic Vox", added: 0, removed: 0, reordered: false }], reordered: [], removed: [] },
      native_stems: [],
    };
    expect(summaryLines(plan({ changes: same })).map((line) => line.kind)).toEqual(["nothing"]);
  });

  it("names the playlists when only one or two are chosen", () => {
    const one = plan({ playlists: [{ id: "11", name: "Friday warm-up" }] });
    expect(summaryLines(one)[0]?.text).toBe("tracks added, across Friday warm-up");
  });
});

describe("space", () => {
  it("formats sizes as the DJ reads them", () => {
    expect(formatBytes(96_400_000_000)).toBe("96.4 GB");
    expect(formatBytes(512_000_000)).toBe("512 MB");
    expect(formatBytes(1_500)).toBe("2 kB");
  });

  it("says the space before and after, against the size when it is known", () => {
    expect(spaceLine(plan().space, 128_000_000_000)).toBe("Free now 40.0 GB of 128 GB · after this load 37.0 GB");
    expect(spaceLine(plan().space, null)).toBe("Free now 40.0 GB · after this load 37.0 GB");
  });

  it("says by how much a load does not fit, and the load is refused", () => {
    const tight = plan({ space: { free_before: 1_000_000_000, needed_bytes: 3_200_000_000, free_after: null, shortfall_bytes: 2_200_000_000, native_stems_bytes: 0, approximate: true } });
    expect(shortfallLine(tight.space)).toBe("Not enough space: this load needs 2.2 GB more than the Stick has free.");
    expect(loadBlocked(tight, 3)).toBe("Not enough space: this load needs 2.2 GB more than the Stick has free.");
    expect(shortfallLine(plan().space)).toBeNull();
  });
});

describe("loadBlocked", () => {
  it("allows a load that fits, with playlists chosen and a plan", () => {
    expect(loadBlocked(plan(), 3)).toBeNull();
  });

  it("refuses with no playlists chosen", () => {
    expect(loadBlocked(plan(), 0)).toBe("Choose at least one playlist to load.");
  });

  it("says why the service cannot plan a load for this Stick", () => {
    expect(loadBlocked(plan({ plan: null, not_loadable: "The Stick's database is in WAL mode." }), 3))
      .toBe("This Stick cannot be loaded: The Stick's database is in WAL mode.");
  });

  it("refuses a Stick with an unfinished write", () => {
    expect(loadBlocked(plan({ stick: serviceStick({ unfinished_write: true }) }), 3))
      .toMatch(/did not finish/);
  });
});

describe("lastLoadedLine", () => {
  const now = new Date("2026-11-13T12:00:00Z");
  it("says when wreckordbox last loaded the Stick", () => {
    expect(lastLoadedLine("2026-11-07 21:04:00.000 +00:00", now)).toBe("Last loaded Nov 07");
    expect(lastLoadedLine("2025-03-01 10:00:00.000 +00:00", now)).toBe("Last loaded Mar 01 2025");
  });
  it("says when it never has", () => {
    expect(lastLoadedLine(null, now)).toBe("Never loaded by wreckordbox");
  });
});

describe("chosenAfterToggle", () => {
  it("adds and removes a playlist, keeping the tree's order", () => {
    const order = ["a", "b", "c"];
    expect(chosenAfterToggle(["c"], "a", order)).toEqual(["a", "c"]);
    expect(chosenAfterToggle(["a", "c"], "a", order)).toEqual(["c"]);
  });
});

describe("fromServiceLoad", () => {
  const report = (over: Record<string, unknown> = {}) => fromServiceLoad({
    stick: serviceStick({ tracks: 3, playlists: 1 }), outcome: "incomplete", error: { code: "x", message: "One Native stems set was not written." },
    planned: serviceDryRun().changes,
    written: { tracks: [{ id: "1", title: "Alpha" }], playlists: ["Friday warm-up"] },
    not_written: { tracks: [{ id: "2", title: "Beta" }], playlists: [] },
    unchanged: { tracks: [{ id: "3", title: "Gamma" }], playlists: ["Melodic Vox"] },
    native_stems: [
      { track: { id: "1", title: "Alpha" }, outcome: "written", reason: null, stems_fact: 4 },
      { track: { id: "2", title: "Beta" }, outcome: "failed", reason: "Its Stems changed since the dry run.", stems_fact: null },
    ],
    ...over,
  });

  it("reads what a load wrote, did not write and found unchanged, and what the Stick holds now", () => {
    expect(report()).toMatchObject({
      outcome: "incomplete",
      error: "One Native stems set was not written.",
      written: { tracks: ["Alpha"], playlists: ["Friday warm-up"] },
      notWritten: { tracks: ["Beta"], playlists: [] },
      unchanged: { tracks: ["Gamma"], playlists: ["Melodic Vox"] },
      holds: { tracks: 3, playlists: 1 },
      nativeStems: [
        { title: "Alpha", outcome: "written", reason: null },
        { title: "Beta", outcome: "failed", reason: "Its Stems changed since the dry run." },
      ],
    });
  });

  it("says what changed and what the Stick holds as different things", () => {
    expect(outcomeLines(report())).toEqual([
      "Written: 1 track, 1 playlist",
      "Already as planned: 1 track, 1 playlist",
      "Not written: 1 track",
      "GIGSTICK now holds 3 tracks in 1 playlist",
    ]);
  });

  it("a load that failed before its first write says nothing was written, not that nothing needed writing", () => {
    const failed = report({
      outcome: "failed", error: { code: "service_failed", message: "The load stopped." },
      written: { tracks: [], playlists: [] },
      not_written: { tracks: [{ id: "1", title: "Alpha" }, { id: "2", title: "Beta" }], playlists: ["Friday warm-up"] },
      unchanged: { tracks: [], playlists: [] },
      native_stems: [],
    });
    const lines = outcomeLines(failed);
    expect(lines[0]).toBe("Nothing was written");
    expect(lines).not.toContain("Nothing needed writing");
    expect(lines).toContain("Not written: 2 tracks, 1 playlist");
  });

  it("a repeat load says nothing needed writing and still says what the Stick holds", () => {
    const repeat = report({
      outcome: "loaded", error: null,
      written: { tracks: [], playlists: [] }, not_written: { tracks: [], playlists: [] },
      unchanged: { tracks: [{ id: "1", title: "Alpha" }, { id: "3", title: "Gamma" }], playlists: ["Melodic Vox"] },
      native_stems: [],
    });
    expect(outcomeLines(repeat)).toEqual([
      "Nothing needed writing",
      "Already as planned: 2 tracks, 1 playlist",
      "GIGSTICK now holds 3 tracks in 1 playlist",
    ]);
  });
});
