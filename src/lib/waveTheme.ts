/**
 * The colours the chosen waveform theme draws in.
 *
 * 3Band, RGB and BLUE are rekordbox's measured colours and never change. The
 * Theme palette takes three colours from the desktop (Omarchy) theme: its
 * blue for the low band, its magenta for the mid and its bright foreground
 * for the core. They are read from CSS custom properties on the document's
 * root, each with its own fallback, so this needs nothing of whatever sets
 * them: with no theme roles at all the Theme palette is Tokyo Night's.
 *
 * The canvases are handed the result (`WaveColours`) and are otherwise the
 * same under every waveform theme. Cue, playhead, beat and phrase colours
 * are not in it.
 */
import { useSyncExternalStore } from "react";

import { MEASURED_COLOURS, themeWaveColours, type WaveColours, type WavePalette } from "@/canvas/waveform";

/**
 * The custom properties the Theme palette reads, by the band each colours.
 * Named as the theme roles are (`--role-*`, `docs/design/tokens.md`): the
 * theme's `blue`, its `magenta` and its `bright_foreground`.
 */
export const THEME_WAVE_ROLES = {
  low: "--role-blue",
  mid: "--role-magenta",
  high: "--role-text-bright",
} as const;

/** Tokyo Night's blue, magenta and bright foreground (`docs/design/tokens.md`). */
export const THEME_WAVE_FALLBACK = { low: "#7aa2f7", mid: "#bb9af7", high: "#c0caf5" } as const;

const byte = (n: number) => n.toString(16).padStart(2, "0");

/**
 * A custom property's colour as `#rrggbb`, or null when it is not one this
 * reads: hex in three, six or eight digits, or `rgb()`/`rgba()` with whole
 * channels. Alpha is dropped; a waveform band is opaque.
 */
export function cssHex(value: string): string | null {
  const text = value.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/.exec(text)?.[1];
  if (hex) return hex.length === 3 ? `#${[...hex].map((c) => c + c).join("")}` : `#${hex.slice(0, 6)}`;
  const rgb = /^rgba?\(\s*(\d{1,3})[\s,]+(\d{1,3})[\s,]+(\d{1,3})\s*(?:[,/]\s*[\d.]+%?\s*)?\)$/.exec(text);
  if (!rgb) return null;
  const channels = [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])];
  return channels.every((c) => c <= 255) ? `#${channels.map(byte).join("")}` : null;
}

let held: { key: string; colours: WaveColours } | null = null;

/**
 * The colours for `palette`, reading the theme's roles through `read` (a
 * custom property's name to its value, empty when unset). The same object
 * comes back while the colours are the same, so it can key an effect.
 */
export function waveColoursFrom(palette: WavePalette, read: (name: string) => string): WaveColours {
  if (palette !== "theme") return MEASURED_COLOURS;
  const role = (band: keyof typeof THEME_WAVE_ROLES) => cssHex(read(THEME_WAVE_ROLES[band])) ?? THEME_WAVE_FALLBACK[band];
  const low = role("low");
  const mid = role("mid");
  const high = role("high");
  const key = `${low}${mid}${high}`;
  if (held?.key !== key) held = { key, colours: themeWaveColours(low, mid, high) };
  return held.colours;
}

/** A short name for a set of colours, for a cache key. */
export function waveColoursKey(colours: WaveColours): string {
  return colours === MEASURED_COLOURS ? "measured" : colours.map((c) => c.map(byte).join("")).join("");
}

function readRoot(name: string): string {
  if (typeof document === "undefined") return "";
  return getComputedStyle(document.documentElement).getPropertyValue(name);
}

const listeners = new Set<() => void>();
let observer: MutationObserver | null = null;
/** The Theme colours as last read from the root; null when they must be read again. */
let read: WaveColours | null = null;

/** Tells every waveform the desktop theme's colours may have changed. */
export function waveThemeChanged(): void {
  read = null;
  for (const listener of listeners) listener();
}

/**
 * Watches for the theme's roles changing. Whatever sets them does it by
 * touching the root element's attributes or a stylesheet in the head, and
 * either is seen here; it may also call `waveThemeChanged()` itself.
 */
function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (!observer && typeof MutationObserver !== "undefined" && typeof document !== "undefined") {
    read = null;
    observer = new MutationObserver(waveThemeChanged);
    observer.observe(document.documentElement, { attributes: true });
    observer.observe(document.head, { childList: true, subtree: true, characterData: true, attributes: true });
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      observer?.disconnect();
      observer = null;
      read = null;
    }
  };
}

/**
 * The root's colours for `palette`. Read from the computed style once per
 * change while anything is watching, not once per waveform: a list has a
 * preview in every row.
 */
function current(palette: WavePalette): WaveColours {
  if (palette !== "theme") return MEASURED_COLOURS;
  if (!observer) return waveColoursFrom(palette, readRoot);
  read ??= waveColoursFrom(palette, readRoot);
  return read;
}

/** The colours to hand the waveform drawing for the chosen waveform theme. */
export function useWaveColours(palette: WavePalette): WaveColours {
  return useSyncExternalStore(
    subscribe,
    () => current(palette),
    () => waveColoursFrom(palette, () => ""),
  );
}
