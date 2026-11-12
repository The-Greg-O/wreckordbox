import { describe, expect, it } from "vitest";
import {
  EMPTY_SHELL_FILTER,
  NO_TRACK,
  dropStale,
  isFiltering,
  keyChoicePicked,
  keyChoices,
  keyIdentity,
  pickSummaries,
  setBpmRange,
  setEnergyRange,
  setTagsAny,
  toCliArgs,
  toServiceSelection,
  toViewFilter,
  toggleKeyChoice,
  togglePrepColour,
  toggleTag,
  tagsAny,
  type ShellFilter,
} from "./shellTrackFilter";
import cases from "./shellTrackFilter.cases.json";

const CATEGORIES = ["Situation", "Mood", "Sound", "Sub-genre"];

const filter = (picks: Partial<ShellFilter>): ShellFilter => ({ ...EMPTY_SHELL_FILTER, ...picks });

describe("an empty filter", () => {
  it("does not narrow, and sends nothing to either side", () => {
    expect(isFiltering(EMPTY_SHELL_FILTER, NO_TRACK)).toBe(false);
    expect(toViewFilter(EMPTY_SHELL_FILTER, CATEGORIES, NO_TRACK)).toEqual({ query: "", filter: undefined });
    expect(toServiceSelection(EMPTY_SHELL_FILTER, NO_TRACK)).toEqual({});
    expect(toCliArgs(EMPTY_SHELL_FILTER, NO_TRACK)).toEqual([]);
  });

  it("text alone narrows", () => {
    const f = filter({ text: "  midnight " });
    expect(isFiltering(f, NO_TRACK)).toBe(true);
    expect(toViewFilter(f, CATEGORIES, NO_TRACK)).toEqual({ query: "midnight", filter: undefined });
    expect(toServiceSelection(f, NO_TRACK)).toEqual({ search: "midnight" });
    expect(toCliArgs(f, NO_TRACK)).toEqual(["--search", "midnight"]);
  });
});

describe("Energy", () => {
  it("any Energy is no constraint, and 1 to 5 is every track that has one", () => {
    expect(toViewFilter(EMPTY_SHELL_FILTER, CATEGORIES, NO_TRACK).filter).toBeUndefined();
    const oneToFive = setEnergyRange(EMPTY_SHELL_FILTER, 1, 5);
    expect(oneToFive.energy).toEqual({ min: 1, max: 5 });
    expect(toViewFilter(oneToFive, CATEGORIES, NO_TRACK).filter).toEqual({ ratings: [1, 2, 3, 4, 5] });
    expect(toServiceSelection(oneToFive, NO_TRACK)).toEqual({ energies: ["1..5"] });
  });

  it("unrated is its own choice, alone or with a range", () => {
    const unrated = filter({ unrated: true });
    expect(toViewFilter(unrated, CATEGORIES, NO_TRACK).filter).toEqual({ ratings: [0] });
    expect(toServiceSelection(unrated, NO_TRACK)).toEqual({ energies: ["0"] });
    const both = setEnergyRange(unrated, 4, 5);
    expect(toViewFilter(both, CATEGORIES, NO_TRACK).filter).toEqual({ ratings: [0, 4, 5] });
    expect(toCliArgs(both, NO_TRACK)).toEqual(["--energy", "4..5", "--energy", "0"]);
  });

  it("orders a reversed range, and Any clears it", () => {
    expect(setEnergyRange(EMPTY_SHELL_FILTER, 5, 2).energy).toEqual({ min: 2, max: 5 });
    expect(setEnergyRange(setEnergyRange(EMPTY_SHELL_FILTER, 2, 5), null, null).energy).toBeNull();
  });
});

describe("Prep colour", () => {
  it("none is a choice and travels as none", () => {
    const f = togglePrepColour(togglePrepColour(EMPTY_SHELL_FILTER, "none"), "Green");
    expect(toViewFilter(f, CATEGORIES, NO_TRACK).filter).toEqual({ colors: ["none", "Green"] });
    expect(toServiceSelection(f, NO_TRACK)).toEqual({ prep_colours: ["none", "Green"] });
  });
});

describe("keys by identity", () => {
  it("a key on the wheel is one identity whatever its spelling; a name off it is itself", () => {
    expect(keyIdentity("Am")).toBe("8A");
    expect(keyIdentity("8A")).toBe("8A");
    expect(keyIdentity("1m")).toBe("8A");
    expect(keyIdentity("Mystery")).toBe("Mystery");
  });

  it("a key picked in one list means the same key in another", () => {
    // Picked in a list holding only "Am".
    const [only] = keyChoices([{ value: "Am", count: 2 }], [], "alphanumeric", "auto");
    const f = toggleKeyChoice(EMPTY_SHELL_FILTER, only!);
    expect(f.keys).toEqual(["8A"]);
    // Shown in a list holding three spellings: one choice, picked.
    const all = keyChoices([{ value: "Am", count: 1 }, { value: "8A", count: 1 }, { value: "1m", count: 1 }], f.keys, "alphanumeric", "auto");
    expect(all).toHaveLength(1);
    expect(keyChoicePicked(f, all[0]!)).toBe(true);
    // Sent as the identity; both sides take every spelling.
    expect(toViewFilter(f, CATEGORIES, NO_TRACK).filter).toEqual({ keyIdentities: ["8A"] });
    expect(toServiceSelection(f, NO_TRACK)).toEqual({ keys: ["8A"] });
  });

  it("a picked key stays offered, and can be unpicked, when the list does not hold it", () => {
    const f = filter({ keys: ["9A", "Mystery"] });
    const choices = keyChoices([{ value: "Am", count: 3 }], f.keys, "alphanumeric", "auto");
    expect(choices.map((c) => [c.label, c.count])).toEqual([["8A", 3], ["9A", 0], ["Mystery", 0]]);
    expect(toggleKeyChoice(f, choices[1]!).keys).toEqual(["Mystery"]);
  });

  it("are written and ordered as the list writes and sorts keys", () => {
    const offered = [
      { value: "Em", count: 2 }, { value: "C", count: 4 }, { value: "Am", count: 3 }, { value: "Unknown", count: 1 },
    ];
    expect(keyChoices(offered, [], "alphanumeric", "auto").map((c) => c.label)).toEqual(["8A", "8B", "9A", "Unknown"]);
    expect(keyChoices(offered, [], "classic", "auto").map((c) => c.label)).toEqual(["Am", "C", "Em", "Unknown"]);
    expect(keyChoices(offered, [], "classic", "musical").map((c) => c.label)).toEqual(["Am", "C", "Em", "Unknown"]);
    expect(keyChoices(offered, [], "alphanumeric", "alphabetical").map((c) => c.label)).toEqual(["8A", "8B", "9A", "Unknown"]);
  });
});

describe("Key match and BPM ±6% follow the loaded track", () => {
  const loaded = { key: "Am", bpmX100: 12500 };

  it("do nothing with no track loaded", () => {
    const f = filter({ keyMatch: true, bpmMatch: true });
    expect(isFiltering(f, NO_TRACK)).toBe(false);
    expect(toViewFilter(f, CATEGORIES, NO_TRACK).filter).toBeUndefined();
    expect(toServiceSelection(f, NO_TRACK)).toEqual({});
  });

  it("Key match is the keys that light against the loaded key", () => {
    const f = filter({ keyMatch: true });
    // 8A: same, 7A and 9A, 8B, and the +2 lift to 10A.
    expect(toViewFilter(f, CATEGORIES, loaded).filter).toEqual({ keyIdentities: ["7A", "8A", "8B", "9A", "10A"] });
    // Another track loaded: the keys follow it.
    expect(toViewFilter(f, CATEGORIES, { key: "C", bpmX100: 12500 }).filter?.keyIdentities).toEqual(["7B", "8A", "8B", "9B", "10B"]);
  });

  it("with keys picked too, both must hold", () => {
    const f = filter({ keyMatch: true, keys: ["9A", "3A"] });
    expect(toViewFilter(f, CATEGORIES, loaded).filter).toEqual({ keyIdentities: ["9A"] });
    const none = filter({ keyMatch: true, keys: ["3A"] });
    expect(toViewFilter(none, CATEGORIES, loaded).filter).toEqual({ keyIdentities: [] });
  });

  it("a loaded track with no key on the wheel leaves Key match inert", () => {
    expect(toViewFilter(filter({ keyMatch: true }), CATEGORIES, { key: "Mystery", bpmX100: 12500 }).filter).toBeUndefined();
  });

  it("BPM ±6% is the band around the loaded BPM, in hundredths", () => {
    const f = filter({ bpmMatch: true });
    // 125.00 ± 7.50.
    expect(toViewFilter(f, CATEGORIES, loaded).filter).toEqual({
      bpm: { values: [], tolerancePct: 0, masterBpmX100: null, rangesX100: [[11750, 13250]] },
    });
    expect(toServiceSelection(f, loaded)).toEqual({ bpm_ranges: ["117.50..132.50"] });
  });

  it("BPM ±6% and a BPM range together are their overlap, or nothing", () => {
    const both = setBpmRange(filter({ bpmMatch: true }), 130, 140);
    expect(toViewFilter(both, CATEGORIES, loaded).filter?.bpm?.rangesX100).toEqual([[12950, 13250]]);
    const apart = setBpmRange(filter({ bpmMatch: true }), 150, 160);
    expect(toViewFilter(apart, CATEGORIES, loaded).filter?.bpm?.rangesX100).toEqual([[1, 0]]);
    expect(toServiceSelection(apart, loaded).bpm_ranges).toEqual(["0.01..0.00"]);
  });
});

describe("BPM ranges", () => {
  it("are whole BPMs as the list rounds them: 124 is 123.50 to 124.49", () => {
    const f = setBpmRange(EMPTY_SHELL_FILTER, 124, 126);
    expect(toViewFilter(f, CATEGORIES, NO_TRACK).filter).toEqual({
      bpm: { values: [], tolerancePct: 0, masterBpmX100: null, rangesX100: [[12350, 12649]] },
    });
    expect(toServiceSelection(f, NO_TRACK)).toEqual({ bpm_ranges: ["123.50..126.49"] });
    expect(toCliArgs(f, NO_TRACK)).toEqual(["--bpm-range", "123.50..126.49"]);
  });

  it("open ends run from 0.50 and up to 999.49; reversed ends are ordered", () => {
    expect(toServiceSelection(setBpmRange(EMPTY_SHELL_FILTER, null, 100), NO_TRACK).bpm_ranges).toEqual(["0.50..100.49"]);
    expect(toServiceSelection(setBpmRange(EMPTY_SHELL_FILTER, 140, null), NO_TRACK).bpm_ranges).toEqual(["139.50..999.49"]);
    expect(setBpmRange(EMPTY_SHELL_FILTER, 130, 120).bpm).toEqual({ min: 120, max: 130 });
    expect(setBpmRange(setBpmRange(EMPTY_SHELL_FILTER, 1, 2), null, null).bpm).toBeNull();
  });
});

describe("MyTags", () => {
  it("send the category's position to the window and its name to the service", () => {
    const f = toggleTag(toggleTag(EMPTY_SHELL_FILTER, "Situation", "Tension builder"), "Situation", "Peak time");
    expect(toViewFilter(f, CATEGORIES, NO_TRACK).filter).toEqual({
      tags: [{ category: 0, any: false, tags: ["Tension builder", "Peak time"] }],
    });
    expect(toServiceSelection(f, NO_TRACK)).toEqual({
      my_tag_picks: [{ category: "Situation", tags: ["Tension builder", "Peak time"], any: false }],
    });
  });

  it("a category's All/Any is kept on its own, before and after its tags", () => {
    const anyFirst = setTagsAny(EMPTY_SHELL_FILTER, "Mood", true);
    const picked = toggleTag(toggleTag(anyFirst, "Mood", "Dark"), "Mood", "Dreamy");
    expect(tagsAny(picked, "Mood")).toBe(true);
    expect(toViewFilter(picked, CATEGORIES, NO_TRACK).filter?.tags).toEqual([{ category: 1, any: true, tags: ["Dark", "Dreamy"] }]);
    const emptied = toggleTag(toggleTag(picked, "Mood", "Dark"), "Mood", "Dreamy");
    expect(emptied.myTags).toEqual([]);
    expect(tagsAny(emptied, "Mood")).toBe(true);
    // Another category's mode is its own.
    expect(tagsAny(emptied, "Situation")).toBe(false);
  });

  it("go to the CLI as a category and a tag, so either may hold a colon", () => {
    const f = setTagsAny(toggleTag(EMPTY_SHELL_FILTER, "Mood:Type", "Dark: very"), "Mood:Type", true);
    expect(toCliArgs(f, NO_TRACK)).toEqual(["--mytag-in", "Mood:Type", "Dark: very", "--mytag-any", "Mood:Type"]);
  });

  it("a tag or category the Library no longer holds is dropped and named", () => {
    let f = toggleTag(toggleTag(EMPTY_SHELL_FILTER, "Situation", "Peak"), "Situation", "Warm-up");
    f = setTagsAny(f, "Situation", true);
    f = toggleTag(f, "Weather", "Rain");
    const { filter: kept, dropped } = dropStale(f, [{ name: "Situation", tags: ["Peak"] }]);
    expect(kept.myTags).toEqual([{ category: "Situation", tags: ["Peak"] }]);
    expect(tagsAny(kept, "Situation")).toBe(true);
    expect(dropped).toEqual(["Situation: Warm-up", "Weather"]);
    // Nothing stale: the same filter back, nothing named.
    expect(dropStale(kept, [{ name: "Situation", tags: ["Peak"] }])).toEqual({ filter: kept, dropped: [] });
  });

  it("a category the Library no longer has points past the end in the window", () => {
    const f = toggleTag(EMPTY_SHELL_FILTER, "Weather", "Rain");
    expect(toViewFilter(f, CATEGORIES, NO_TRACK).filter?.tags).toEqual([{ category: 4, any: false, tags: ["Rain"] }]);
  });
});

describe("what the bar says", () => {
  it("names each active pick", () => {
    let f = toggleTag(toggleTag(EMPTY_SHELL_FILTER, "Situation", "Tension builder"), "Situation", "Peak time");
    f = setEnergyRange(f, 3, 5);
    expect(pickSummaries(f, NO_TRACK)).toEqual({
      myTags: { Situation: "Tension builder, Peak time" },
      energy: "3-5",
      bpm: null,
      keys: null,
      prepColours: null,
    });
    expect(pickSummaries(filter({ unrated: true }), NO_TRACK).energy).toBe("unrated");
    expect(pickSummaries(setEnergyRange(filter({ unrated: true }), 4, 4), NO_TRACK).energy).toBe("4, unrated");
    expect(pickSummaries(setBpmRange(EMPTY_SHELL_FILTER, 124, null), NO_TRACK).bpm).toBe("124+");
    expect(pickSummaries(setBpmRange(EMPTY_SHELL_FILTER, null, 128), NO_TRACK).bpm).toBe("-128");
    expect(pickSummaries(filter({ prepColours: ["none", "Green"] }), NO_TRACK).prepColours).toBe("None, Green");
    expect(pickSummaries(filter({ keys: ["8A", "Mystery"] }), NO_TRACK, (k) => (k === "8A" ? "Am" : k)).keys).toBe("Am, Mystery");
    expect(pickSummaries(setTagsAny(toggleTag(toggleTag(EMPTY_SHELL_FILTER, "Mood", "Dark"), "Mood", "Dreamy"), "Mood", true), NO_TRACK).myTags)
      .toEqual({ Mood: "Dark or Dreamy" });
  });
});

describe("the cases both sides are checked against", () => {
  for (const c of cases.cases) {
    it(c.name, () => {
      const f = { ...EMPTY_SHELL_FILTER, ...(c.filter as Partial<ShellFilter>) };
      const loaded = (c as { loaded?: { key: string; bpmX100: number } }).loaded ?? NO_TRACK;
      const window = toViewFilter(f, cases.categories, loaded);
      expect({ query: window.query, ...(window.filter ? { filter: window.filter } : {}) }).toEqual(c.window);
      expect(toServiceSelection(f, loaded)).toEqual(c.service);
    });
  }
});
