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

let held: { key: string; colours: WaveColours } | null = null;

/**
 * The colours for `palette` under `theme`. The same object comes back while
 * the colours are the same, so it can key an effect.
 */
export function waveColoursFrom(palette: WaveformTheme, theme: Pick<Theme, "wave">): WaveColours {
  if (palette !== "theme") return MEASURED_COLOURS;
  const { low, mid, high } = themeWave(theme);
  const key = `${low}${mid}${high}`;
  if (held?.key !== key) held = { key, colours: themeWaveColours(low, mid, high) };
  return held.colours;
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
function useShownTheme(): Theme {
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

/** The colours to hand the waveform drawing for `palette`. */
export function useWaveColours(palette: WaveformTheme): WaveColours {
  const theme = useShownTheme();
  return waveColoursFrom(palette, theme);
}
