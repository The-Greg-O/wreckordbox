import { describe, expect, it } from "vitest";
import {
  EMPTY_SHELL_FILTER,
  isFiltering,
  pickSummaries,
  setBpmRange,
  setEnergyRange,
  toCliArgs,
  toServiceSelection,
  toViewFilter,
  toggleKey,
  togglePrepColour,
  toggleTag,
  setTagsAny,
  type ShellFilter,
} from "./shellTrackFilter";
import cases from "./shellTrackFilter.cases.json";

const CATEGORIES = ["Situation", "Mood", "Sound", "Sub-genre"];

/** The design's example: Situation Tension builder and Peak time, Energy 3 to 5. */
const designExample: ShellFilter = {
  ...EMPTY_SHELL_FILTER,
  myTags: [{ category: "Situation", tags: ["Tension builder", "Peak time"], any: false }],
  energy: { min: 3, max: 5 },
};

describe("an empty filter", () => {
  it("does not narrow, and sends nothing to either side", () => {
    expect(isFiltering(EMPTY_SHELL_FILTER)).toBe(false);
    expect(toViewFilter(EMPTY_SHELL_FILTER, CATEGORIES)).toEqual({ query: "", filter: undefined });
    expect(toServiceSelection(EMPTY_SHELL_FILTER)).toEqual({});
    expect(toCliArgs(EMPTY_SHELL_FILTER)).toEqual([]);
  });

  it("text alone narrows", () => {
    const f = { ...EMPTY_SHELL_FILTER, text: "  midnight " };
    expect(isFiltering(f)).toBe(true);
    expect(toViewFilter(f, CATEGORIES)).toEqual({ query: "midnight", filter: undefined });
    expect(toServiceSelection(f)).toEqual({ search: "midnight" });
    expect(toCliArgs(f)).toEqual(["--search", "midnight"]);
  });
});

describe("the window's list filter", () => {
  it("sends Energy as the ratings in the range and MyTags by the category's position", () => {
    expect(toViewFilter(designExample, CATEGORIES).filter).toEqual({
      ratings: [3, 4, 5],
      tags: [{ category: 0, any: false, tags: ["Tension builder", "Peak time"] }],
    });
  });

  it("sends a BPM range as every whole BPM in it, open ends from 1 and up to 999", () => {
    const closed = setBpmRange(EMPTY_SHELL_FILTER, 124, 126);
    expect(toViewFilter(closed, CATEGORIES).filter).toEqual({
      bpm: { values: [124, 125, 126], tolerancePct: 0, masterBpmX100: null },
    });
    const from = toViewFilter(setBpmRange(EMPTY_SHELL_FILTER, 998, null), CATEGORIES).filter;
    expect(from?.bpm?.values).toEqual([998, 999]);
    const upTo = toViewFilter(setBpmRange(EMPTY_SHELL_FILTER, null, 3), CATEGORIES).filter;
    expect(upTo?.bpm?.values).toEqual([1, 2, 3]);
  });

  it("sends keys and Prep colours by name", () => {
    const f = togglePrepColour(toggleKey(toggleKey(EMPTY_SHELL_FILTER, "8A"), "9A"), "Green");
    expect(toViewFilter(f, CATEGORIES).filter).toEqual({ keys: ["8A", "9A"], colors: ["Green"] });
  });

  it("points a category the Library no longer has past the end, so it narrows to nothing", () => {
    const f = toggleTag(EMPTY_SHELL_FILTER, "Weather", "Rain");
    expect(toViewFilter(f, CATEGORIES).filter?.tags).toEqual([{ category: 4, any: false, tags: ["Rain"] }]);
  });
});

describe("the service's track query", () => {
  it("is the same picks in tracks.query's own words", () => {
    const f: ShellFilter = {
      text: "night",
      myTags: [
        { category: "Situation", tags: ["Peak time"], any: true },
        { category: "Sound", tags: ["Vocal"], any: false },
      ],
      energy: { min: 3, max: 5 },
      prepColours: ["Green", "Red"],
      bpm: { min: 124, max: 128 },
      keys: ["8A"],
    };
    expect(toServiceSelection(f, "123")).toEqual({
      search: "night",
      playlist: "123",
      keys: ["8A"],
      prep_colours: ["Green", "Red"],
      energy: "3..5",
      // Whole BPMs 124 to 128 are 123.50 to 128.49, as the window buckets them.
      bpm: "123.50..128.49",
      my_tag_picks: [
        { category: "Situation", tags: ["Peak time"], any: true },
        { category: "Sound", tags: ["Vocal"], any: false },
      ],
    });
    expect(toCliArgs(f, "123")).toEqual([
      "--search", "night",
      "--playlist", "123",
      "--key", "8A",
      "--prep-colour", "Green", "--prep-colour", "Red",
      "--energy", "3..5",
      "--bpm", "123.50..128.49",
      "--mytag-in", "Situation:Peak time", "--mytag-any", "Situation",
      "--mytag-in", "Sound:Vocal",
    ]);
  });

  it("gives open BPM ends the window's own bounds", () => {
    expect(toServiceSelection(setBpmRange(EMPTY_SHELL_FILTER, null, 100)).bpm).toBe("0.50..100.49");
    expect(toServiceSelection(setBpmRange(EMPTY_SHELL_FILTER, 140, null)).bpm).toBe("139.50..999.49");
  });
});

describe("editing", () => {
  it("toggles a tag in and out, dropping a category left empty", () => {
    const one = toggleTag(EMPTY_SHELL_FILTER, "Mood", "Dark");
    const two = toggleTag(one, "Mood", "Dreamy");
    expect(two.myTags).toEqual([{ category: "Mood", tags: ["Dark", "Dreamy"], any: false }]);
    expect(toggleTag(toggleTag(two, "Mood", "Dark"), "Mood", "Dreamy").myTags).toEqual([]);
  });

  it("switches a category between all and any", () => {
    const f = setTagsAny(toggleTag(EMPTY_SHELL_FILTER, "Mood", "Dark"), "Mood", true);
    expect(f.myTags[0]?.any).toBe(true);
  });

  it("orders a reversed range and drops a range covering everything", () => {
    expect(setEnergyRange(EMPTY_SHELL_FILTER, 5, 2).energy).toEqual({ min: 2, max: 5 });
    expect(setEnergyRange(designExample, 1, 5).energy).toBeNull();
    expect(setBpmRange(EMPTY_SHELL_FILTER, 130, 120).bpm).toEqual({ min: 120, max: 130 });
    expect(setBpmRange(designExample, null, null).bpm).toBeNull();
  });

  it("toggling a key twice leaves no key", () => {
    expect(toggleKey(toggleKey(EMPTY_SHELL_FILTER, "8A"), "8A").keys).toEqual([]);
  });
});

describe("what the bar says", () => {
  it("names each active pick as the design does", () => {
    expect(pickSummaries(designExample)).toEqual({
      myTags: { Situation: "Tension builder, Peak time" },
      energy: "3-5",
      bpm: null,
      keys: null,
      prepColours: null,
    });
    const f = setBpmRange(togglePrepColour(toggleKey(EMPTY_SHELL_FILTER, "8A"), "Green"), 124, null);
    expect(pickSummaries(f)).toMatchObject({ bpm: "124+", keys: "8A", prepColours: "Green" });
    expect(pickSummaries(setBpmRange(EMPTY_SHELL_FILTER, null, 128)).bpm).toBe("-128");
    expect(pickSummaries(setEnergyRange(EMPTY_SHELL_FILTER, 4, 4)).energy).toBe("4");
  });
});


/** The cases file writes a long run of whole BPMs as `"lo..hi"`. */
function expandBpm(window: { query: string; filter?: Record<string, unknown> }) {
  const bpm = window.filter?.bpm as { values: number[] | string } | undefined;
  if (bpm && typeof bpm.values === "string") {
    const [lo, hi] = bpm.values.split("..").map(Number) as [number, number];
    return { ...window, filter: { ...window.filter, bpm: { ...bpm, values: Array.from({ length: hi - lo + 1 }, (_, i) => lo + i) } } };
  }
  return window;
}

describe("the cases both sides are checked against", () => {
  for (const c of cases.cases) {
    it(c.name, () => {
      const filter = c.filter as ShellFilter;
      const window = toViewFilter(filter, cases.categories);
      expect({ query: window.query, ...(window.filter ? { filter: window.filter } : {}) }).toEqual(expandBpm(c.window));
      expect(toServiceSelection(filter)).toEqual(c.service);
    });
  }
});
