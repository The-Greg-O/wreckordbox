import { describe, expect, it } from "vitest";

import { crumbTally } from "./crumbTally";

describe("crumbTally", () => {
  it("reads as the design's breadcrumb: count, then hours and minutes", () => {
    // docs/design/screens/mock.html: "CURRENT / Melodic Vox 30 · 2h24m".
    expect(crumbTally({ tracks: 30, seconds: 2 * 3600 + 24 * 60 + 41, bytes: 1 })).toBe("30 · 2h24m");
  });

  it("pads the minutes after an hour and leaves them alone under one", () => {
    expect(crumbTally({ tracks: 12, seconds: 3600 + 5 * 60, bytes: 1 })).toBe("12 · 1h05m");
    expect(crumbTally({ tracks: 3, seconds: 18 * 60 + 59, bytes: 1 })).toBe("3 · 18m");
  });

  it("says only the count of an empty playlist", () => {
    expect(crumbTally({ tracks: 0, seconds: 0, bytes: 0 })).toBe("0");
  });

  it("says nothing for a list that has no tally (All Tracks, a Stick's playlist)", () => {
    expect(crumbTally(undefined)).toBe("");
  });

  it("does not trust a broken length", () => {
    expect(crumbTally({ tracks: 2, seconds: Number.NaN, bytes: 0 })).toBe("2 · 0m");
  });
});
