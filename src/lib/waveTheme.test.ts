import { describe, expect, it } from "vitest";

import { MEASURED_COLOURS } from "@/canvas/waveform";
import { THEME_WAVE_FALLBACK, THEME_WAVE_ROLES, cssHex, waveColoursFrom } from "./waveTheme";

/** A stand-in for the document's computed style: role name to value. */
const roles = (values: Record<string, string>) => (name: string) => values[name] ?? "";

describe("the Theme waveform's colours", () => {
  it("come from the theme's blue, magenta and bright foreground", () => {
    const colours = waveColoursFrom("theme", roles({
      [THEME_WAVE_ROLES.low]: "#89b4fa",
      [THEME_WAVE_ROLES.mid]: " #cba6f7 ",
      [THEME_WAVE_ROLES.high]: "rgb(205, 214, 244)",
    }));
    // Low alone, mid alone, all three: the outer envelope, the band inside it, the core.
    expect([colours[1], colours[2], colours[7]]).toEqual([[137, 180, 250], [203, 166, 247], [205, 214, 244]]);
  });

  it("fall back to Tokyo Night where the theme gives no role, one at a time", () => {
    expect(THEME_WAVE_FALLBACK).toEqual({ low: "#7aa2f7", mid: "#bb9af7", high: "#c0caf5" });
    const none = waveColoursFrom("theme", roles({}));
    expect([none[1], none[2], none[7]]).toEqual([[122, 162, 247], [187, 154, 247], [192, 202, 245]]);
    const partial = waveColoursFrom("theme", roles({ [THEME_WAVE_ROLES.mid]: "#ff00ff", [THEME_WAVE_ROLES.high]: "not a colour" }));
    expect([partial[1], partial[2], partial[7]]).toEqual([[122, 162, 247], [255, 0, 255], [192, 202, 245]]);
  });

  it("are rekordbox's measured ones for every other waveform theme, whatever the desktop theme says", () => {
    const loud = roles({ [THEME_WAVE_ROLES.low]: "#ff0000", [THEME_WAVE_ROLES.mid]: "#00ff00", [THEME_WAVE_ROLES.high]: "#0000ff" });
    for (const palette of ["3band", "rgb", "blue"] as const) {
      expect(waveColoursFrom(palette, loud)).toBe(MEASURED_COLOURS);
    }
  });

  it("keeps the same object while the theme's colours stay the same", () => {
    const read = roles({ [THEME_WAVE_ROLES.low]: "#89b4fa" });
    expect(waveColoursFrom("theme", read)).toBe(waveColoursFrom("theme", read));
  });

  it("reads the forms a custom property can hold", () => {
    expect(cssHex("#ABC")).toBe("#aabbcc");
    expect(cssHex("#7AA2F7")).toBe("#7aa2f7");
    expect(cssHex("#7aa2f7ff")).toBe("#7aa2f7");
    expect(cssHex("rgb(1 2 3)")).toBe("#010203");
    expect(cssHex("rgba(255, 128, 0, 0.5)")).toBe("#ff8000");
    expect(cssHex("")).toBeNull();
    expect(cssHex("blue")).toBeNull();
    expect(cssHex("rgb(300, 0, 0)")).toBeNull();
  });
});
