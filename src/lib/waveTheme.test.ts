import { describe, expect, it } from "vitest";

import { MEASURED_COLOURS } from "@/canvas/waveform";
import { THEME_WAVE_FALLBACK, effectiveWaveform, fileWaveform, hexRgb, legibleOn, waveColoursFrom } from "./waveTheme";

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

describe("the Theme waveform against its well", () => {
  const contrast = (a: readonly number[], b: readonly number[]) => {
    const lum = (c: readonly number[]) => {
      const [r, g, bl] = c.map((v) => { const s = v / 255; return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; });
      return 0.2126 * r! + 0.7152 * g! + 0.0722 * bl!;
    };
    const [x, y] = [lum(a), lum(b)];
    return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
  };

  it("keeps a palette that already stands out exactly as it is", () => {
    expect(legibleOn("#7aa2f7", [0, 0, 0])).toBe("#7aa2f7");
    expect(legibleOn("#1e66f5", [255, 255, 255])).toBe("#1e66f5");
  });

  it("lifts a colour that would vanish into the well until it stands out at least 3:1", () => {
    // A light theme's bright foreground is black; the well stays black.
    const lifted = legibleOn("#000000", [0, 0, 0]);
    expect(contrast(hexRgb(lifted), [0, 0, 0])).toBeGreaterThanOrEqual(3);
    // A low-contrast palette: every band a shade off the well.
    for (const band of ["#0e0e14", "#1a1b26", "#24283b"]) {
      expect(contrast(hexRgb(legibleOn(band, [14, 14, 20])), [14, 14, 20])).toBeGreaterThanOrEqual(3);
    }
    // On a light well it is darkened instead.
    expect(contrast(hexRgb(legibleOn("#f0f0f0", [255, 255, 255])), [255, 255, 255])).toBeGreaterThanOrEqual(3);
  });

  it("draws every band of the Theme palette legible on the well it is given, and leaves the measured ones alone", () => {
    const theme = { wave: { low: "#000000", mid: "#050505", high: "#0a0a0a" } };
    const colours = waveColoursFrom("theme", theme, [0, 0, 0]);
    for (const band of [colours[1]!, colours[2]!, colours[7]!]) expect(contrast(band, [0, 0, 0])).toBeGreaterThanOrEqual(3);
    expect(waveColoursFrom("3band", theme, [0, 0, 0])).toBe(MEASURED_COLOURS);
  });
});
