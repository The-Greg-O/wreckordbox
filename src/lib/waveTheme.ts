/**
 * The waveform colour theme in force, and the colours it draws in.
 *
 * One source of truth for which theme every waveform draws (old layout,
 * compact player, Intake's large player, the rows): `useWaveformTheme()`.
 * The DJ's theme file (`~/.config/wreckordbox/theme.toml`, key `waveform`)
 * wins where it sets one; otherwise it is the persisted view preference
 * (Preferences › View › Color, or the deck's menu).
 *
 * 3Band, RGB and BLUE are rekordbox's measured colours and never change.
 * The Theme palette takes three colours from the Omarchy theme, as the app
 * shell reads them from the palette itself (`wave` in the theme payload):
 * its blue for the low band, its magenta for the mid and its bright
 * foreground for the core. Never the window's roles, so neither the theme
 * file's colours nor anything else that recolours the chrome moves a
 * waveform. With no Omarchy theme (a browser tab, `follow = "none"`, another
 * system) the Theme palette is Tokyo Night's.
 *
 * The canvases are handed the result (`WaveColours`) and are otherwise the
 * same under every waveform theme. Cue, playhead, beat and phrase colours
 * are not in it.
 */
import { useSyncExternalStore } from "react";

import { MEASURED_COLOURS, themeWaveColours, type WaveColours } from "@/canvas/waveform";
import type { WaveformTheme } from "@/lib/preferences";
import { shownTheme, subscribeShownTheme, type Theme, type ThemeWave } from "@/lib/themeRoles";
import { usePreferences } from "@/store/usePreferences";

/** Tokyo Night's blue, magenta and bright foreground. */
export const THEME_WAVE_FALLBACK: ThemeWave = { low: "#7aa2f7", mid: "#ad8ee6", high: "#c0caf5" };

export const WAVEFORM_THEME_NAMES: readonly WaveformTheme[] = ["3band", "rgb", "blue", "theme"];

const HEX = /^#[0-9a-f]{6}$/i;
const byte = (n: number) => n.toString(16).padStart(2, "0");

/** The Omarchy theme's three colours, or the fallback, band by band. */
export function themeWave(theme: Pick<Theme, "wave">): ThemeWave {
  const own = theme.wave;
  const band = (name: keyof ThemeWave) => {
    const value = own?.[name];
    return typeof value === "string" && HEX.test(value) ? value.toLowerCase() : THEME_WAVE_FALLBACK[name];
  };
  return { low: band("low"), mid: band("mid"), high: band("high") };
}

export type Rgb = readonly [number, number, number];

/** `#rrggbb` as its three channels. */
export function hexRgb(hex: string): Rgb {
  const n = Number.parseInt(hex.slice(1, 7), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function luminance([r, g, b]: Rgb): number {
  const linear = (v: number) => {
    const s = v / 255;
    return s <= 0.040_45 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
}

/** WCAG contrast ratio, 1 (none) to 21. */
export function contrastRatio(one: Rgb, other: Rgb): number {
  const [a, b] = [luminance(one), luminance(other)];
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

/**
 * What a band must stand out from its well by (WCAG's figure for graphics),
 * and what an adjusted band is taken to. The targets rise from the low band
 * to the core so that bands lifted off the same well stay apart.
 */
export const WELL_CONTRAST = 3;
const WELL_TARGETS = { low: 3, mid: 4, high: 5.5 } as const;

/**
 * `hex`, or where it stands out from `well` by less than `least`, `hex`
 * moved towards white or black (whichever the well is further from) until
 * it stands out by `target`. A theme's own colours are kept wherever they
 * can be seen: a light theme's black foreground on the black well of the
 * player, or a low-contrast palette's shades of its own ground, are not.
 */
export function legibleOn(hex: string, well: Rgb, least = WELL_CONTRAST, target = least): string {
  const colour = hexRgb(hex);
  if (contrastRatio(colour, well) >= least) return hex;
  const toward: Rgb = contrastRatio([255, 255, 255], well) >= contrastRatio([0, 0, 0], well) ? [255, 255, 255] : [0, 0, 0];
  let mixed: Rgb = colour;
  for (let step = 1; step <= 20; step += 1) {
    const t = step / 20;
    mixed = [0, 1, 2].map((i) => Math.round(colour[i]! + (toward[i]! - colour[i]!) * t)) as unknown as Rgb;
    if (contrastRatio(mixed, well) >= target) break;
  }
  return `#${mixed.map(byte).join("")}`;
}

/** At most this many sets of colours are held (a well per kind of strip and row). */
const HELD_MOST = 32;
const held = new Map<string, WaveColours>();

/**
 * The colours for `palette` under `theme`, each Theme band legible on
 * `well` (the colour behind the canvas; black where unknown). The same
 * object comes back while the colours are the same, so it can key an
 * effect.
 */
export function waveColoursFrom(palette: WaveformTheme, theme: Pick<Theme, "wave">, well: Rgb = [0, 0, 0]): WaveColours {
  if (palette !== "theme") return MEASURED_COLOURS;
  const own = themeWave(theme);
  const low = legibleOn(own.low, well, WELL_CONTRAST, WELL_TARGETS.low);
  const mid = legibleOn(own.mid, well, WELL_CONTRAST, WELL_TARGETS.mid);
  const high = legibleOn(own.high, well, WELL_CONTRAST, WELL_TARGETS.high);
  const key = `${low}${mid}${high}`;
  let colours = held.get(key);
  if (!colours) {
    colours = themeWaveColours(low, mid, high);
    if (held.size >= HELD_MOST) held.clear();
    held.set(key, colours);
  }
  return colours;
}

/**
 * The colour a canvas is drawn over: the background of the nearest element
 * around it that paints one. Black where none does.
 */
export function wellOf(element: Element | null): Rgb {
  for (let at = element?.parentElement ?? null; at; at = at.parentElement) {
    const match = /rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)(?:[\s,/]+([\d.]+))?/.exec(getComputedStyle(at).backgroundColor);
    if (match && (match[4] === undefined || Number(match[4]) > 0.5)) {
      return [Number(match[1]), Number(match[2]), Number(match[3])];
    }
  }
  return [0, 0, 0];
}

/** `#rrggbb` of a well, for a cache key. */
export function wellKey(well: Rgb): string {
  return well.map(byte).join("");
}

/** A short name for a set of colours, for a cache key. */
export function waveColoursKey(colours: WaveColours): string {
  return colours === MEASURED_COLOURS ? "measured" : colours.map((c) => c.map(byte).join("")).join("");
}

/** The theme file's waveform theme, if it names one this window knows. */
export function fileWaveform(theme: Pick<Theme, "waveform">): WaveformTheme | null {
  const named = theme.waveform;
  return typeof named === "string" && (WAVEFORM_THEME_NAMES as readonly string[]).includes(named)
    ? (named as WaveformTheme)
    : null;
}

/** The waveform theme in force: the theme file's, else the preference. */
export function effectiveWaveform(
  preferred: WaveformTheme,
  theme: Pick<Theme, "waveform">,
): { palette: WaveformTheme; fromFile: boolean } {
  const fromFile = fileWaveform(theme);
  return fromFile ? { palette: fromFile, fromFile: true } : { palette: preferred, fromFile: false };
}

/** The theme this window shows, kept current. */
export function useShownTheme(): Theme {
  return useSyncExternalStore(subscribeShownTheme, shownTheme, shownTheme);
}

/** `effectiveWaveform` of `preferred` and the theme this window shows, kept current. */
export function useEffectiveWaveform(preferred: WaveformTheme): { palette: WaveformTheme; fromFile: boolean } {
  return effectiveWaveform(preferred, useShownTheme());
}

/** The waveform theme every waveform draws, and whether the theme file set it. */
export function useWaveformTheme(): { palette: WaveformTheme; fromFile: boolean } {
  return useEffectiveWaveform(usePreferences().view.waveformColor);
}

/** The colours to hand the waveform drawing for `palette`, on `well`. */
export function useWaveColours(palette: WaveformTheme, well?: Rgb): WaveColours {
  const theme = useShownTheme();
  return waveColoursFrom(palette, theme, well);
}
