import { afterEach, describe, expect, it } from "vitest";

import { backingSize, exactBackingSize, MAX_RATIO, MAX_WIDTH, STEP } from "./canvasSize";
import { setPixelZoom } from "./pixelRatio";

describe("backingSize", () => {
  it("follows the element rather than a fixed number", () => {
    expect(backingSize(400, 14, 1).width).toBe(400);
    expect(backingSize(1200, 14, 1).width).toBe(1200);
  });

  it("doubles for a retina display", () => {
    expect(backingSize(400, 14, 2)).toEqual({ width: 800, height: 28 });
  });

  it("stops at 2x, because a 14-pixel strip cannot show more", () => {
    expect(backingSize(400, 14, 3)).toEqual(backingSize(400, 14, MAX_RATIO));
  });

  it("quantises the width so a drag does not reallocate on every pixel", () => {
    const a = backingSize(401, 14, 1).width;
    const b = backingSize(400 + STEP - 1, 14, 1).width;
    expect(a).toBe(b);
    expect(a % STEP).toBe(0);
  });

  it("never asks for a buffer a layout bug could make enormous", () => {
    expect(backingSize(100_000, 14, 2).width).toBe(MAX_WIDTH);
  });

  it("survives a zero-sized or absent measurement", () => {
    expect(backingSize(0, 0, 1)).toEqual({ width: STEP, height: 1 });
    expect(backingSize(Number.NaN, 14, Number.NaN).width).toBeGreaterThan(0);
  });
});

describe("exactBackingSize", () => {
  it("is the element's device pixels, not a step wider", () => {
    // backingSize would give 2,368pt here: 28 device pixels squeezed away.
    expect(exactBackingSize(2354, 133, 2)).toEqual({ width: 4708, height: 266 });
    expect(exactBackingSize(1177, 133, 1)).toEqual({ width: 1177, height: 133 });
  });

  it("keeps backingSize's ratio cap, width cap and bad-measurement floor", () => {
    expect(exactBackingSize(400, 14, 3)).toEqual(exactBackingSize(400, 14, MAX_RATIO));
    expect(exactBackingSize(100_000, 14, 2).width).toBe(MAX_WIDTH);
    expect(exactBackingSize(0, 0, 1)).toEqual({ width: 1, height: 1 });
    expect(exactBackingSize(Number.NaN, 14, Number.NaN).width).toBeGreaterThan(0);
  });
});

describe("under the whole-interface scale", () => {
  afterEach(() => setPixelZoom(1));

  it("is the device pixels a scaled-up element covers, past 2x on a retina display", () => {
    // 1.25 on a 2x display: 2.5 device pixels to the CSS pixel.
    setPixelZoom(1.25);
    expect(exactBackingSize(1000, 100, 2 * 1.25)).toEqual({ width: 2500, height: 250 });
    expect(backingSize(400, 16, 2 * 1.25)).toEqual({ width: 1000, height: 40 });
  });

  it("is the device pixels a scaled-down element covers, below 1x", () => {
    // 0.8 on a 1x display: a 1000pt strip is 800 device pixels wide.
    setPixelZoom(0.8);
    expect(exactBackingSize(1000, 100, 0.8)).toEqual({ width: 800, height: 80 });
    expect(backingSize(400, 20, 0.8)).toEqual({ width: 320, height: 16 });
  });

  it("still stops at twice the display's pixels", () => {
    setPixelZoom(1.5);
    expect(exactBackingSize(1000, 100, 3 * 1.5)).toEqual(exactBackingSize(1000, 100, MAX_RATIO * 1.5));
  });
});
