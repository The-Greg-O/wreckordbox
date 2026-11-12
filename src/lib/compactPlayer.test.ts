import { describe, expect, it } from "vitest";

import { jumpSizeById } from "./player";
import {
  COMPACT_JUMP_BEATS,
  COMPACT_JUMP_DEFAULT,
  compactJumpBeats,
  compactJumpSizeId,
  compactLength,
  compactPosition,
} from "./compactPlayer";

describe("the compact player's beat jump sizes", () => {
  it("offers 4, 8, 16 and 32 beats, in that order", () => {
    expect(COMPACT_JUMP_BEATS).toEqual([4, 8, 16, 32]);
  });

  it("maps each size to the deck's jump size of that many beats", () => {
    expect(compactJumpSizeId(4)).toBe("4beats");
    expect(compactJumpSizeId(8)).toBe("8beats");
    expect(compactJumpSizeId(16)).toBe("16beats");
    // rekordbox's menu calls 32 beats "8Bars".
    expect(compactJumpSizeId(32)).toBe("8bars");
    for (const beats of COMPACT_JUMP_BEATS) expect(jumpSizeById(compactJumpSizeId(beats)).beats).toBe(beats);
  });

  it("reads a stored size, and anything else as the default", () => {
    expect(compactJumpBeats(8)).toBe(8);
    expect(compactJumpBeats(32)).toBe(32);
    expect(COMPACT_JUMP_DEFAULT).toBe(16);
    for (const stored of [undefined, null, 0, 2, 64, "8", 8.5, Number.NaN]) {
      expect(compactJumpBeats(stored), String(stored)).toBe(16);
    }
  });
});

describe("the compact player's position readout", () => {
  it("shows minutes, seconds and tenths elapsed", () => {
    expect(compactPosition(0)).toBe("0:00.0");
    expect(compactPosition(83.46)).toBe("1:23.4");
    expect(compactPosition(600.05)).toBe("10:00.0");
  });

  it("shows the length in minutes and seconds", () => {
    expect(compactLength(348)).toBe("5:48");
    expect(compactLength(59.9)).toBe("0:59");
    expect(compactLength(0)).toBe("0:00");
  });

  it("never shows a negative or unreadable time", () => {
    expect(compactPosition(-1)).toBe("0:00.0");
    expect(compactPosition(Number.NaN)).toBe("0:00.0");
    expect(compactLength(Number.POSITIVE_INFINITY)).toBe("0:00");
  });
});
