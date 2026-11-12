/**
 * The Library list's Key cells fit at every font size the theme file
 * accepts (8 to 24 px), lit and unlit, in Camelot and Classic notation
 * (wbx-45.10). The theme file's sizes reach the window as `theme-file.spec.ts`
 * announces them; lights come from the loaded track (`camelot-keys.spec.ts`).
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { player } from "./compactPlayer";
import { DEFAULT_PREFERENCES } from "../src/lib/preferences";
import { BROWSER_THEME_EVENT, type Theme, type ThemeRoles } from "../src/lib/themeRoles";

const TOKYO_NIGHT: ThemeRoles = {
  mode: "dark", ground: "#0e0e14", panel: "#13141c", working: "#1a1b26", hairline: "#24283b",
  selection: "#292e42", border: "#414868", text: "#a9b1d6", textDim: "#565f89", textBright: "#c0caf5",
  accent: "#7aa2f7", onAccent: "#0e0e14", tabFrom: "#bb9af7", tabTo: "#ff9e64",
};

/** The fixture's one value that is not a key ("Unknown"): shown raw, and may be cut. */
const NOT_A_KEY = "Key light 09";

async function open(page: Page, keyDisplay: "camelot" | "classic"): Promise<Locator> {
  await page.addInitScript((full) => {
    localStorage.setItem("rbl.preferences", JSON.stringify(full));
  }, { ...DEFAULT_PREFERENCES, advanced: { ...DEFAULT_PREFERENCES.advanced, protectLibrary: false },
    view: { ...DEFAULT_PREFERENCES.view, viewShell: true, keyDisplay } });
  await page.goto("/?writable=1&keyLights=1");
  await page.getByRole("treeitem", { name: "All Tracks", exact: true }).click();
  const list = page.getByRole("tabpanel");
  const first = list.getByRole("row").filter({ hasText: "Key light 01" });
  await first.locator('[data-col="title"]').dblclick();
  await expect(player(page).getByTestId("compact-player-title")).toHaveText("Key light 01");
  return list;
}

async function announce(page: Page, theme: Theme): Promise<void> {
  await page.evaluate(
    ([name, detail]) => void window.dispatchEvent(new CustomEvent(name as string, { detail })),
    [BROWSER_THEME_EVENT, theme] as const,
  );
}

/** Key cells whose drawn content (text, badge and mark) leaves the cell's content box. */
async function overflowing(cells: Locator): Promise<string[]> {
  return cells.evaluateAll((elements) => elements.flatMap((element) => {
    const text = (element.textContent ?? "").trim();
    if (!text) return [];
    const range = document.createRange();
    range.selectNodeContents(element);
    const ink = range.getBoundingClientRect();
    // Across, inside the cell's padding: a value running into it touches the
    // next column's rule. Down, inside the row.
    const outer = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    const left = outer.left + Number.parseFloat(style.paddingLeft) + Number.parseFloat(style.borderLeftWidth);
    const right = outer.right - Number.parseFloat(style.paddingRight) - Number.parseFloat(style.borderRightWidth);
    const box = { left, right, width: right - left, top: outer.top, bottom: outer.bottom, height: outer.height };
    const fits = ink.left >= box.left - 0.5 && ink.right <= box.right + 0.5 && ink.top >= box.top - 0.5 && ink.bottom <= box.bottom + 0.5;
    return fits ? [] : [`"${text}" ${Math.round(ink.width)}x${Math.round(ink.height)} in ${Math.round(box.width)}x${Math.round(box.height)}`];
  }));
}

const SIZES: { text: number; mono: number }[] = [
  { text: 8, mono: 8 }, { text: 12, mono: 12 }, { text: 16, mono: 16 }, { text: 24, mono: 24 },
  { text: 8, mono: 24 }, { text: 24, mono: 8 },
];

/**
 * The theme file names any face for numbers: a narrow monospace, the DJ's
 * own (JetBrains Mono) and a wide serif that is not monospace at all. A face
 * this machine lacks falls back, which only makes that case easier.
 */
const FACES = ["DejaVu Sans Mono", "JetBrainsMono Nerd Font", "C059"];

for (const keyDisplay of ["camelot", "classic"] as const) {
  for (const { text, mono } of SIZES) for (const face of FACES) {
    test(`${keyDisplay} key cells fit, lit and unlit, at ${text} px text and ${mono} px numbers in ${face}`, async ({ page }) => {
      await page.setViewportSize({ width: 2600, height: 1200 });
      const list = await open(page, keyDisplay);
      await announce(page, {
        roles: TOKYO_NIGHT, font: "DejaVu Sans Mono", interfaceFont: "DejaVu Sans", interfaceFontSize: text,
        monoFont: face, monoFontSize: mono,
      });
      const keys = list.getByRole("row").filter({ hasNotText: NOT_A_KEY }).locator('[role="gridcell"][data-col="key"]');
      // Both lights and an unlit key are on show, so all three are measured.
      await expect(keys.getByRole("img", { name: /Harmonic match/ }).first()).toBeVisible();
      await expect(keys.getByRole("img", { name: /Energy boost/ }).first()).toBeVisible();
      await expect(keys.filter({ hasText: keyDisplay === "camelot" ? "6A" : "Gm" })).toHaveCount(1);
      await expect(keys.filter({ hasText: keyDisplay === "camelot" ? "8A" : "Am" }).first()).toHaveCSS("font-size", `${mono}px`);
      expect(await overflowing(keys)).toEqual([]);
    });
  }
}
