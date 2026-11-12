import { describe, expect, it } from "vitest";

import { padTime } from "./IntakeCueRow";

describe("padTime", () => {
  it("prints minutes, seconds and tenths as the design's pads do", () => {
    expect(padTime(400)).toBe("0:00.4");
    expect(padTime(32_100)).toBe("0:32.1");
    expect(padTime(160_900)).toBe("2:40.9");
  });

  it("drops what is under a tenth rather than rounding up past the cue", () => {
    expect(padTime(59_999)).toBe("0:59.9");
    expect(padTime(600_000)).toBe("10:00.0");
  });

  it("never prints a negative time", () => {
    expect(padTime(-50)).toBe("0:00.0");
  });
});
