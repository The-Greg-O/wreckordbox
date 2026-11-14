import { describe, expect, it } from "vitest";

import { beatMs, nextShown, nowDetail, standInBands, sweepMs } from "./loudLoad";

describe("beatMs: the display moves in time with the track", () => {
  it("is one beat of the track's tempo", () => {
    expect(beatMs(12_000)).toBe(500);
    expect(beatMs(12_800)).toBeCloseTo(468.75);
  });

  it("folds a very slow or very fast tempo into the dance range", () => {
    expect(beatMs(6_000)).toBe(500); // 60 BPM moves at 120
    expect(beatMs(34_000)).toBeCloseTo(352.94, 1); // 340 BPM moves at 170
  });

  it("moves at 120 BPM when the track has no tempo", () => {
    expect(beatMs(0)).toBe(500);
    expect(beatMs(null)).toBe(500);
    expect(beatMs(Number.NaN)).toBe(500);
  });
});

describe("sweepMs: how long the write head takes across a track", () => {
  it("guesses two seconds before any track has been seen to finish", () => {
    expect(sweepMs([])).toBe(2000);
  });

  it("is the median time the recent tracks took", () => {
    expect(sweepMs([1000, 5000, 3000])).toBe(3000);
    expect(sweepMs([1000, 9000, 3000, 5000])).toBe(4000);
  });

  it("follows the last few tracks, not the whole load", () => {
    expect(sweepMs([60_000, 60_000, 60_000, 1000, 1000, 1000, 1000, 1000])).toBe(1000);
  });

  it("stays between a third of a second and twenty seconds", () => {
    expect(sweepMs([5])).toBe(300);
    expect(sweepMs([600_000])).toBe(20_000);
  });
});

describe("nowDetail: the line under the track's title", () => {
  it("names the artist, the tempo and the key in Camelot", () => {
    expect(nowDetail({ artist: "Someone", bpmX100: 12_800, key: "Am" })).toBe("Someone · 128 BPM · 8A");
    expect(nowDetail({ artist: "Someone", bpmX100: 12_850, key: "8A" })).toBe("Someone · 128.5 BPM · 8A");
  });

  it("leaves out what the track does not have", () => {
    expect(nowDetail({ artist: "", bpmX100: 0, key: "" })).toBe("");
    expect(nowDetail({ artist: "", bpmX100: 17_400, key: "" })).toBe("174 BPM");
  });
});

describe("standInBands: the shape drawn while a track has no waveform to show", () => {
  it("is three bands a column", () => {
    expect(standInBands("x", 200)).toHaveLength(600);
  });

  it("is the same for the same track and different for another", () => {
    expect(standInBands("One", 100)).toEqual(standInBands("One", 100));
    expect(standInBands("One", 100)).not.toEqual(standInBands("Two", 100));
  });

  it("has a body: no band column is all silence", () => {
    const bands = standInBands("One", 300);
    for (let column = 0; column < 300; column += 1) {
      expect(bands[column * 3]! + bands[column * 3 + 1]!).toBeGreaterThan(0);
    }
  });
});

describe("the setting (view.loudStickLoad)", () => {
  it("is on unless the DJ turned it off, and an odd stored value leaves it on", async () => {
    const { sanitisePreferences } = await import("./preferences");
    expect(sanitisePreferences({}).view.loudStickLoad).toBe(true);
    expect(sanitisePreferences({ view: { loudStickLoad: false } }).view.loudStickLoad).toBe(false);
    expect(sanitisePreferences({ view: { loudStickLoad: "no" } }).view.loudStickLoad).toBe(true);
  });
});

describe("nextShown: the display changes track at most every so often", () => {
  it("shows a new track at once when the last change was long enough ago", () => {
    expect(nextShown({ shown: "a", since: 0 }, "b", 1000, 400)).toEqual({ show: { shown: "b", since: 1000 }, retryIn: null });
  });

  it("keeps the track it shows when the last change was too recent, and says when to look again", () => {
    expect(nextShown({ shown: "a", since: 900 }, "b", 1000, 400)).toEqual({ show: { shown: "a", since: 900 }, retryIn: 300 });
  });

  it("changes nothing when the latest track is the one shown", () => {
    expect(nextShown({ shown: "a", since: 900 }, "a", 1000, 400)).toEqual({ show: { shown: "a", since: 900 }, retryIn: null });
  });
});
