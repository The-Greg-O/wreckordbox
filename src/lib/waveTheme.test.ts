import { describe, expect, it } from "vitest";

import { MEASURED_COLOURS } from "@/canvas/waveform";
import { THEME_WAVE_FALLBACK, effectiveWaveform, fileWaveform, waveColoursFrom } from "./waveTheme";

describe("the Theme waveform's colours", () => {
  it("come from the Omarchy theme's blue, magenta and bright foreground", () => {
    const colours = waveColoursFrom("theme", { wave: { low: "#89b4fa", mid: "#CBA6F7", high: "#cdd6f4" } });
    // Low alone, mid alone, all three: the outer envelope, the band inside it, the core.
    expect([colours[1], colours[2], colours[7]]).toEqual([[137, 180, 250], [203, 166, 247], [205, 214, 244]]);
  });

  it("fall back to Tokyo Night where there is no Omarchy theme, band by band", () => {
    expect(THEME_WAVE_FALLBACK).toEqual({ low: "#7aa2f7", mid: "#ad8ee6", high: "#c0caf5" });
    const none = waveColoursFrom("theme", {});
    expect([none[1], none[2], none[7]]).toEqual([[122, 162, 247], [173, 142, 230], [192, 202, 245]]);
    const partial = waveColoursFrom("theme", { wave: { low: "blue", mid: "#ff00ff", high: "" } });
    expect([partial[1], partial[2], partial[7]]).toEqual([[122, 162, 247], [255, 0, 255], [192, 202, 245]]);
  });

  it("are rekordbox's measured ones for every other waveform theme, whatever the Omarchy theme says", () => {
    const loud = { wave: { low: "#ff0000", mid: "#00ff00", high: "#0000ff" } };
    for (const palette of ["3band", "rgb", "blue"] as const) {
      expect(waveColoursFrom(palette, loud)).toBe(MEASURED_COLOURS);
    }
  });

  it("keeps the same object while the theme's colours stay the same", () => {
    const theme = { wave: { low: "#89b4fa", mid: "#cba6f7", high: "#cdd6f4" } };
    expect(waveColoursFrom("theme", theme)).toBe(waveColoursFrom("theme", { ...theme }));
  });
});

describe("the waveform theme in force", () => {
  it("is the preference while the theme file names none", () => {
    expect(effectiveWaveform("rgb", {})).toEqual({ palette: "rgb", fromFile: false });
    expect(effectiveWaveform("theme", { waveform: null })).toEqual({ palette: "theme", fromFile: false });
  });

  it("is the theme file's where it names one: the file wins", () => {
    expect(effectiveWaveform("rgb", { waveform: "blue" })).toEqual({ palette: "blue", fromFile: true });
    expect(effectiveWaveform("3band", { waveform: "theme" })).toEqual({ palette: "theme", fromFile: true });
  });

  it("ignores a name it does not know", () => {
    expect(fileWaveform({ waveform: "rainbow" })).toBeNull();
    expect(effectiveWaveform("rgb", { waveform: "rainbow" })).toEqual({ palette: "rgb", fromFile: false });
  });
});
