import { describe, expect, it } from "vitest";

import { EMPTY_SHELL_FILTER, type ShellFilter } from "@/lib/shellTrackFilter";
import { suggestedPlaylistName, toSaveRequest } from "@/lib/saveFilterPlaylist";

const designExample: ShellFilter = {
  ...EMPTY_SHELL_FILTER,
  myTags: [{ category: "Situation", tags: ["Tension builder", "Peak time"] }],
  energy: { min: 3, max: 5 },
};

describe("toSaveRequest", () => {
  it("sends the filter in tracks.query's terms, with nothing for All Tracks", () => {
    expect(toSaveRequest(designExample, null)).toEqual({
      energies: ["3..5"],
      my_tag_picks: [{ category: "Situation", tags: ["Tension builder", "Peak time"], any: false }],
    });
  });

  it("names the open list when it is not All Tracks", () => {
    expect(toSaveRequest({ ...EMPTY_SHELL_FILTER, prepColours: ["Green"] }, "Bar test")).toEqual({
      prep_colours: ["Green"],
      within: "Bar test",
    });
  });

  it("says Key match and BPM ±6% are on instead of freezing the loaded track's keys and tempo", () => {
    const f: ShellFilter = { ...EMPTY_SHELL_FILTER, keyMatch: true, bpmMatch: true, keys: ["8A"] };
    expect(toSaveRequest(f, null)).toEqual({ keys: ["8A"], follows: ["key", "bpm"] });
  });

  it("keeps the text, which the save refuses with its reason", () => {
    expect(toSaveRequest({ ...EMPTY_SHELL_FILTER, text: " track 01 " }, null)).toEqual({ search: "track 01" });
  });
});

describe("suggestedPlaylistName", () => {
  it("says what is picked, as the bar's controls say it", () => {
    expect(suggestedPlaylistName(designExample, (k) => k)).toBe("Tension builder, Peak time · Energy 3-5");
    expect(
      suggestedPlaylistName({ ...EMPTY_SHELL_FILTER, keys: ["8A"], bpm: { min: 122, max: 126 }, prepColours: ["none"] }, (k) => `key ${k}`),
    ).toBe("Prep None · BPM 122-126 · key 8A");
  });

  it("has a plain name when nothing is picked", () => {
    expect(suggestedPlaylistName(EMPTY_SHELL_FILTER, (k) => k)).toBe("Saved filter");
  });
});
