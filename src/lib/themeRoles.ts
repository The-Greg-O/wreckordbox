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
  /** The two ends of the active view tab's line. */
  tabFrom: string;
  tabTo: string;
};

/** What the app shell reports: `theme_roles` and the `theme:changed` event. */
export type Theme = {
  /** Null when no usable theme is present. */
  roles: ThemeRoles | null;
  /** The system monospace family; null when the system names none. */
  font: string | null;
};

const COLOUR_PROPERTIES: Record<Exclude<keyof ThemeRoles, "mode">, string> = {
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
  tabFrom: "--role-tab-from",
  tabTo: "--role-tab-to",
};
const CONTROL_PROPERTY = "--role-control";
const FONT_PROPERTY = "--role-font";

const COLOUR = /^#[0-9a-f]{6}$/i;
const FAMILY = /^[\p{L}\p{N} ._+()-]{1,100}$/u;

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
  properties[CONTROL_PROPERTY] = properties[COLOUR_PROPERTIES.hairline] ?? null;
  properties[FONT_PROPERTY] = theme.font && FAMILY.test(theme.font) ? `"${theme.font}", ui-monospace, monospace` : null;
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
  if (theme.roles) root.setAttribute("data-theme-mode", theme.roles.mode === "light" ? "light" : "dark");
  else root.removeAttribute("data-theme-mode");
}

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * Shows the current theme in this window and every later one, as the DJ
 * switches. Called from `main.tsx` for every window before it draws.
 */
export function followTheme(): void {
  if (!isTauri) return;
  void (async () => {
    try {
      const [{ invoke }, { listen }] = await Promise.all([import("@tauri-apps/api/core"), import("@tauri-apps/api/event")]);
      await listen<Theme>("theme:changed", (event) => applyTheme(event.payload));
      applyTheme(await invoke<Theme>("theme_roles"));
    } catch (failure) {
      console.warn("the window could not read the theme", failure);
    }
  })();
}
