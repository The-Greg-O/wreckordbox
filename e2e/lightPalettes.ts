/**
 * Omarchy's stock palettes as the app shell reports them, and announcing one
 * to the mock interface the way the app shell does (`followTheme()` hears
 * the event). Shared by the light-theme specs (wbx-45.1, wbx-lfa).
 */
import type { Page } from "@playwright/test";
import { BROWSER_THEME_EVENT, type Theme, type ThemeRoles } from "../src/lib/themeRoles";

export const LATTE: ThemeRoles = {
  mode: "light", ground: "#d7d8dc", panel: "#e3e4e8", working: "#eff1f5", hairline: "#dce0e8",
  selection: "#ccd0da", border: "#acb0be", text: "#4c4f69", textDim: "#9ca0b0", textBright: "#4c4f69",
  accent: "#1e66f5", onAccent: "#ffffff", tabFrom: "#ea76cb", tabTo: "#df8e1d",
};
export const WHITE: ThemeRoles = {
  mode: "light", ground: "#e8e8e8", panel: "#f5f5f5", working: "#ffffff", hairline: "#c0c0c0",
  selection: "#c0c0c0", border: "#808080", text: "#000000", textDim: "#c0c0c0", textBright: "#000000",
  accent: "#6e6e6e", onAccent: "#ffffff", tabFrom: "#2e2e2e", tabTo: "#4a4a4a",
};
export const TOKYO_NIGHT: ThemeRoles = {
  mode: "dark", ground: "#0e0e14", panel: "#13141c", working: "#1a1b26", hairline: "#24283b",
  selection: "#292e42", border: "#414868", text: "#a9b1d6", textDim: "#565f89", textBright: "#c0caf5",
  accent: "#7aa2f7", onAccent: "#0e0e14", tabFrom: "#bb9af7", tabTo: "#ff9e64",
};

export async function announce(page: Page, theme: Theme): Promise<void> {
  await page.evaluate(
    ([name, detail]) => void window.dispatchEvent(new CustomEvent(name as string, { detail })),
    [BROWSER_THEME_EVENT, theme] as const,
  );
}

export const rgb = (hex: string): string => {
  const n = Number.parseInt(hex.slice(1), 16);
  return `rgb(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${(n >> 0) & 255})`;
};
