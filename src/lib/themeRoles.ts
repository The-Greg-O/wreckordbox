/**
 * The chrome follows the DJ's Omarchy theme.
 *
 * The app shell reads the active theme's palette and the system monospace
 * font (`src-tauri/src/omarchy_theme.rs`) and reports them as roles; this
 * sets each role as a `--role-*` custom property on the root element, where
 * it overrides the fallback of the same name in `styles/tokens.css`. The
 * chrome tokens (`--c-black`, `--c-text`, `--f-ui` and so on) are defined
 * from the roles, so both the upstream layout and the view shell follow.
 *
 * Colours that carry meaning (waveform bands, cues, phrases, Prep colours,
 * meters) are not roles and nothing here can set them.
 *
 * In a browser tab (the mock build) there is no app shell, so nothing is
 * set and the stylesheet's fallbacks, upstream's own colours, apply.
 */

import { useMemo, useSyncExternalStore } from "react";

/** The roles of `docs/design/tokens.md`, each `#rrggbb`. */
export type ThemeRoles = {
  mode: "dark" | "light";
  /** Deepest ground: top bar, status line, waveform wells. */
  ground: string;
  /** Panels: tree, player, target list. */
  panel: string;
  /** The working area. */
  working: string;
  /** Hairlines and unset pips. */
  hairline: string;
  /** A selected row. */
  selection: string;
  /** Control borders. */
  border: string;
  text: string;
  /** Secondary text and labels. */
  textDim: string;
  /** Emphasised text. */
  textBright: string;
  /** Focus, the active filter, the deck marker. */
  accent: string;
  /** Text and glyphs on a control filled with the accent. */
  onAccent: string;
  /** The two ends of the active view tab's line. */
  tabFrom: string;
  tabTo: string;
  /** A control's fill where it is not the hairline colour (the built-in look under a theme file). */
  control?: string;
  /** A lit control under the pointer, where the look gives its own rather than one derived from the accent. */
  accentHover?: string;
  /** The built-in look with the theme file's keys over it, not a theme: upstream's lit chrome keeps its own colours. */
  builtIn?: boolean;
};

/** A mistake in the DJ's theme file, left out of what is shown. */
export type ThemeProblem = { line: number; message: string };

/** What the app shell reports: `theme_roles` and the `theme:changed` event. */
export type Theme = {
  /** Null when no usable theme is present. */
  roles: ThemeRoles | null;
  /** The face for upstream's layout: the theme file's, else the system monospace family; null when neither names one. */
  font: string | null;
  /** The theme file's interface face and base size (px), for the view shell; absent without a file. */
  interfaceFont?: string | null;
  interfaceFontSize?: number | null;
  /** The theme file's face and size (px) for text whose digits line up. */
  monoFont?: string | null;
  monoFontSize?: number | null;
  /** The theme file's mistakes. */
  problems?: ThemeProblem[];
  /** The Omarchy palette's colours for the Theme waveform (never the theme file's); absent without a usable Omarchy theme. */
  wave?: ThemeWave | null;
  /** The theme file's waveform colour theme, which wins over Preferences; absent when the file sets none. */
  waveform?: string | null;
};

/** The Theme waveform's band colours, each `#rrggbb`: the theme's blue, magenta and bright foreground. */
export type ThemeWave = { low: string; mid: string; high: string };

const COLOUR_PROPERTIES: Record<Exclude<keyof ThemeRoles, "mode" | "control" | "accentHover" | "builtIn">, string> = {
  ground: "--role-ground",
  panel: "--role-panel",
  working: "--role-working",
  hairline: "--role-hairline",
  selection: "--role-selection",
  border: "--role-border",
  text: "--role-text",
  textDim: "--role-text-dim",
  textBright: "--role-text-bright",
  accent: "--role-accent",
  onAccent: "--role-on-accent",
  tabFrom: "--role-tab-from",
  tabTo: "--role-tab-to",
};
const CONTROL_PROPERTY = "--role-control";
const ACCENT_HOVER_PROPERTY = "--role-accent-hover";
const FONT_PROPERTY = "--role-font";
/** Set only by the DJ's theme file (`src-tauri/src/theme_file.rs`); read in `styles/tokens-shell.css`. */
const INTERFACE_FONT_PROPERTY = "--role-font-interface";
const INTERFACE_SIZE_PROPERTY = "--role-font-size";
const MONO_FONT_PROPERTY = "--role-font-mono";
const MONO_SIZE_PROPERTY = "--role-font-mono-size";
/** Set only by the theme file's `font_size`: its ratio to the 12 px base, for upstream's measured sizes. */
const FONT_SCALE_PROPERTY = "--role-font-scale";
/** The view shell's fixed sizes at the theme file's size (`styles/tokens-shell.css`), at 100% scale. */
const SHELL_HEIGHTS: Record<string, number> = {
  "--role-shell-tree-w": 186,
  "--role-shell-row-h": 24,
  "--role-shell-heading-h": 22,
  "--role-shell-status-h": 22,
  "--role-shell-top-bar-h": 30,
};
/** The sizes the theme file may give, in CSS pixels at 100% scale. */
const SMALLEST = 8;
const LARGEST = 24;
/** The interface's base text size without a theme file (`--f-size-shell`). */
export const BASE_FONT_SIZE = 12;

const validSize = (size: number | null | undefined): number | null =>
  typeof size === "number" && Number.isFinite(size) && size >= SMALLEST && size <= LARGEST ? size : null;

/**
 * How much larger than the built-in sizes the theme file makes the text:
 * `text` for the interface, `numbers` for the values whose digits line up
 * (the interface's when the file gives no monospace size). 1 without a file.
 */
export function fontScales(theme: Theme): { text: number; numbers: number } {
  const text = (validSize(theme.interfaceFontSize) ?? BASE_FONT_SIZE) / BASE_FONT_SIZE;
  const mono = validSize(theme.monoFontSize);
  return { text, numbers: mono === null ? text : mono / BASE_FONT_SIZE };
}

const COLOUR = /^#[0-9a-f]{6}$/i;

const channels = (hex: string): number[] => [1, 3, 5].map((at) => Number.parseInt(hex.slice(at, at + 2), 16));
const luminance = (hex: string): number => {
  const [r, g, b] = channels(hex).map((value) => {
    const c = value / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
};

/**
 * A lit control under the pointer: the accent moved 15% away from the colour
 * written on it (toward white under a dark label, toward black under a light
 * one), so the label stays at least as readable as at rest.
 */
function accentHover(accent: string, onAccent: string): string {
  const toward = luminance(onAccent) < luminance(accent) ? 255 : 0;
  return `#${channels(accent).map((value) => Math.round(value + (toward - value) * 0.15).toString(16).padStart(2, "0")).join("")}`;
}
const FAMILY = /^[\p{L}\p{N} ._+()-]{1,100}$/u;

const face = (family: string | null | undefined): string | null =>
  family && FAMILY.test(family) ? `"${family}", ui-monospace, monospace` : null;
const pixels = (size: number | null | undefined): string | null => {
  const valid = validSize(size);
  return valid === null ? null : `${valid}px`;
};

/**
 * Each role's custom property and the value a theme gives it; null where the
 * theme gives none and the stylesheet's fallback is to stand.
 */
export function themeProperties(theme: Theme): Record<string, string | null> {
  const properties: Record<string, string | null> = {};
  for (const [role, property] of Object.entries(COLOUR_PROPERTIES) as [keyof typeof COLOUR_PROPERTIES, string][]) {
    const value = theme.roles?.[role];
    properties[property] = typeof value === "string" && COLOUR.test(value) ? value : null;
  }
  // A control's fill is the hairline colour in a theme; only upstream's
  // fallbacks tell the two apart.
  const own = (value: string | undefined) => (typeof value === "string" && COLOUR.test(value) ? value : null);
  properties[CONTROL_PROPERTY] = own(theme.roles?.control) ?? properties[COLOUR_PROPERTIES.hairline] ?? null;
  const accent = properties[COLOUR_PROPERTIES.accent];
  const onAccent = properties[COLOUR_PROPERTIES.onAccent];
  properties[ACCENT_HOVER_PROPERTY] = own(theme.roles?.accentHover) ?? (accent && onAccent ? accentHover(accent, onAccent) : null);
  properties[FONT_PROPERTY] = face(theme.font);
  properties[INTERFACE_FONT_PROPERTY] = face(theme.interfaceFont);
  properties[INTERFACE_SIZE_PROPERTY] = pixels(theme.interfaceFontSize);
  properties[MONO_FONT_PROPERTY] = face(theme.monoFont);
  properties[MONO_SIZE_PROPERTY] = pixels(theme.monoFontSize);
  const size = validSize(theme.interfaceFontSize);
  const scale = size === null ? null : size / BASE_FONT_SIZE;
  properties[FONT_SCALE_PROPERTY] = scale === null ? null : String(scale);
  for (const [property, height] of Object.entries(SHELL_HEIGHTS)) {
    properties[property] = scale === null ? null : `${Math.ceil(height * scale)}px`;
  }
  return properties;
}

/** As much of the root element as showing a theme needs. */
type Root = {
  style: { setProperty(name: string, value: string): void; removeProperty(name: string): unknown };
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
};

/**
 * Shows `theme` in this window: the roles as custom properties, and the
 * theme's mode as `data-theme-mode` for the few rules that differ by it.
 */
export function applyTheme(theme: Theme, root: Root = document.documentElement): void {
  for (const [property, value] of Object.entries(themeProperties(theme))) {
    if (value === null) root.style.removeProperty(property);
    else root.style.setProperty(property, value);
  }
  // The built-in look keeps upstream's lit chrome (only a theme turns it to
  // the accent), unless the file makes it light, where upstream's layout
  // needs its guard.
  const mode = theme.roles?.mode === "light" ? "light" : "dark";
  if (theme.roles && (!theme.roles.builtIn || mode === "light")) root.setAttribute("data-theme-mode", mode);
  else root.removeAttribute("data-theme-mode");
}

/** Where a window learns the theme from: the app shell, or a stand-in for it. */
export type ThemeSource = {
  /** The theme as it stands. */
  current(): Promise<Theme>;
  /** Calls `listener` with each later theme. */
  onChange(listener: (theme: Theme) => void): Promise<unknown>;
};

/**
 * Shows the source's theme on `root` and every later one. Subscribes before
 * asking, so a switch between the two is not lost.
 */
export async function followThemeFrom(source: ThemeSource, root?: Root): Promise<void> {
  await source.onChange((theme) => show(theme, root));
  show(await source.current(), root);
}

function show(theme: Theme, root?: Root): void {
  applyTheme(theme, root);
  shown = theme;
  for (const listener of [...listeners]) listener();
}

/** The theme this window last showed, for what React draws from it (the theme file's problems). */
let shown: Theme = { roles: null, font: null };
const listeners = new Set<() => void>();

/** For `useSyncExternalStore`: calls `listener` whenever another theme is shown. */
export function subscribeShownTheme(listener: () => void): () => void {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** The theme this window last showed. */
export function shownTheme(): Theme {
  return shown;
}

/** `fontScales` of the theme this window shows, for what lays text out in script (the lists). */
export function useFontScales(): { text: number; numbers: number } {
  const theme = useSyncExternalStore(subscribeShownTheme, shownTheme);
  const { text, numbers } = fontScales(theme);
  return useMemo(() => ({ text, numbers }), [text, numbers]);
}

/** The app shell: the `theme_roles` command and the `theme:changed` event. */
async function appShell(): Promise<ThemeSource> {
  const [{ invoke }, { listen }] = await Promise.all([import("@tauri-apps/api/core"), import("@tauri-apps/api/event")]);
  return {
    current: () => invoke<Theme>("theme_roles"),
    onChange: (listener) => listen<Theme>("theme:changed", (event) => listener(event.payload)),
  };
}

/**
 * In a browser tab there is no app shell and no theme: upstream's colours
 * stand. A page script can still show one, as the browser tests do, by
 * dispatching a `wreckordbox:theme` event on the window with a `Theme` as
 * its detail; it goes through the same path as the app shell's.
 */
export const BROWSER_THEME_EVENT = "wreckordbox:theme";
const browserTab: ThemeSource = {
  current: () => Promise.resolve({ roles: null, font: null }),
  onChange: (listener) => {
    window.addEventListener(BROWSER_THEME_EVENT, (event) => listener((event as CustomEvent<Theme>).detail));
    return Promise.resolve();
  },
};

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * Shows the current theme in this window and every later one, as the DJ
 * switches. Called from `main.tsx` for every window before it draws.
 */
export function followTheme(): void {
  if (typeof window === "undefined") return;
  void (async () => {
    try {
      await followThemeFrom(isTauri ? await appShell() : browserTab);
    } catch (failure) {
      console.warn("the window could not read the theme", failure);
    }
  })();
}
