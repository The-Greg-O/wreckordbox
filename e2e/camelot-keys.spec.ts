import { expect, test, type Page } from "@playwright/test";
import { player, playButton } from "./compactPlayer";
import { DEFAULT_PREFERENCES } from "../src/lib/preferences";
import { BROWSER_THEME_EVENT, type Theme } from "../src/lib/themeRoles";

const row = (page: Page, n: number) => page.getByRole("tabpanel").getByRole("row").filter({
  has: page.getByRole("gridcell", { name: `Key light ${String(n).padStart(2, "0")}`, exact: true }),
});
const key = (page: Page, n: number) => row(page, n).locator('[data-col="key"]');

async function open(page: Page, protect = false, view: Record<string, unknown> = {}) {
  await page.addInitScript((full) => {
    if (sessionStorage.getItem("keys-seeded")) return;
    sessionStorage.setItem("keys-seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify(full));
  }, { ...DEFAULT_PREFERENCES, advanced: { ...DEFAULT_PREFERENCES.advanced, protectLibrary: protect },
    view: { ...DEFAULT_PREFERENCES.view, viewShell: true, ...view } });
  await page.goto("/?writable=1&keyLights=1");
  await page.getByRole("treeitem", { name: "All Tracks", exact: true }).click();
  await expect(row(page, 1)).toBeVisible();
}

async function load(page: Page, n: number) {
  await row(page, n).locator('[data-col="title"]').dblclick();
  await expect(player(page).getByTestId("compact-player-title")).toHaveText(`Key light ${String(n).padStart(2, "0")}`);
}

// Assert the colour actually painted, not the presence of a state attribute.
async function light(page: Page, n: number, colour: "green" | "yellow" | "none") {
  const badge = key(page, n).locator("span").first();
  await expect(badge).toHaveCSS("background-color", {
    green: "rgb(158, 206, 106)", yellow: "rgb(224, 175, 104)", none: "rgba(0, 0, 0, 0)",
  }[colour]);
}

test("Camelot is the default; Classic changes the list and loaded player and persists", async ({ page }) => {
  await open(page);
  await expect(key(page, 1).locator("[data-key-text]")).toHaveText("8A");
  await load(page, 1);
  await expect(player(page).getByTestId("compact-player-key")).toHaveText("8A");
  await page.getByRole("button", { name: "Command palette", exact: true }).click();
  await page.getByRole("combobox", { name: "Search actions" }).fill("preferences");
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Preferences" });
  const choices = dialog.getByRole("radiogroup", { name: "Key display format", exact: true });
  await expect(choices.getByRole("radio", { name: "Camelot", exact: true })).toBeChecked();
  const classic = choices.getByRole("radio", { name: "Classic", exact: true });
  await classic.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Enter");
  await expect(classic).toBeChecked();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(key(page, 1).locator("[data-key-text]")).toHaveText("Am");
  await expect(player(page).getByTestId("compact-player-key")).toHaveText("Am");
  await expect(playButton(page)).toBeVisible();
  await light(page, 5, "yellow");
  await page.reload();
  await expect(key(page, 1).locator("[data-key-text]")).toHaveText("Am");
});

test("lights follow the loaded key, never row selection, and wrap the wheel", async ({ page }) => {
  await open(page);
  await expect(key(page, 1).locator("[data-key-text]")).toHaveText("8A");
  for (let n = 1; n <= 12; n++) await light(page, n, "none");
  await load(page, 1);
  for (const n of [1, 2, 3, 4]) await light(page, n, "green");
  await light(page, 5, "yellow");
  for (const n of [6, 7, 8, 9, 10, 11, 12]) await light(page, n, "none");
  await row(page, 5).locator('[data-col="title"]').click();
  await light(page, 5, "yellow");
  await light(page, 1, "green");
  await load(page, 5);
  await light(page, 1, "none");
  await light(page, 3, "green");
  await light(page, 11, "yellow");
  await load(page, 10); // 11A → 1A wraps for the +2 energy boost.
  await light(page, 12, "yellow");
  await load(page, 8); // Missing loaded key clears everything.
  for (let n = 1; n <= 12; n++) await light(page, n, "none");
});

test("Library Protection refuses a key edit while notation and lights still work", async ({ page }) => {
  await open(page, true);
  await load(page, 1);
  await key(page, 5).dblclick();
  await expect(page.getByTestId("shell-status")).toContainText("Editing is locked by Library Protection");
  await expect(key(page, 5).locator("[data-key-text]")).toHaveText("10A");
  await light(page, 5, "yellow");
  await expect(page.getByRole("combobox", { name: "Key", exact: true })).toHaveCount(0);
});

test("editing a displayed Camelot key saves the stored key through the existing edit and Undo", async ({ page }) => {
  await open(page);
  await load(page, 1);
  await key(page, 6).dblclick();
  const editor = page.getByRole("combobox", { name: "Key", exact: true });
  await editor.selectOption({ label: "10A" });
  await expect(key(page, 6).locator("[data-key-text]")).toHaveText("10A");
  await light(page, 6, "yellow");
  await expect(player(page).getByTestId("compact-player-key")).toHaveText("8A");
  await expect(playButton(page)).toBeVisible();
  await page.evaluate(() => (window as unknown as { __menu: (id: string) => void }).__menu("undo"));
  await expect(key(page, 6).locator("[data-key-text]")).toHaveText("6A");
  await light(page, 6, "none");
});

const LIGHT: Theme = {
  font: "monospace",
  interfaceFont: "sans-serif", interfaceFontSize: 12,
  monoFont: "monospace", monoFontSize: 12,
  roles: {
    mode: "light", ground: "#d7d8dc", panel: "#e3e4e8", working: "#eff1f5", hairline: "#dce0e8",
    selection: "#ccd0da", border: "#acb0be", text: "#4c4f69", textDim: "#9ca0b0", textBright: "#4c4f69",
    accent: "#1e66f5", onAccent: "#ffffff", tabFrom: "#8839ef", tabTo: "#df8e1d",
  },
};

for (const width of [1280, 640, 436]) {
  for (const uiScale of [4, 6]) {
    test(`key lights stay legible at ${width}px, scale ${uiScale}, on dark and light test grounds`, async ({ page }, info) => {
      await open(page, false, { uiScale });
      await load(page, 1);
      // Choose All Tracks while the tree is on screen, then tile the window.
      await page.setViewportSize({ width, height: 900 });
      // wbx-60 owns dropping columns at narrow widths. Until then the
      // existing list scrolls sideways; check our labels once in view.
      await key(page, 5).scrollIntoViewIfNeeded();
      // Isolated ground test: the shell's light chrome is wbx-45.1. Do not
      // mistake an event alone for a changed background. Use actual light
      // and dark cells, then assert their painted ground before contrast.
      await page.addStyleTag({ content: `[data-col="key"][role="gridcell"] { background: #101010 !important; }` });
      for (const theme of ["dark", "light"]) {
        if (theme === "light") await page.evaluate(([event, detail]) => {
          window.dispatchEvent(new CustomEvent(event, { detail }));
        }, [BROWSER_THEME_EVENT, LIGHT] as const);
        if (theme === "light") {
          await expect(page.locator("html")).toHaveAttribute("data-theme-mode", "light");
          await page.addStyleTag({ content: `[data-col="key"][role="gridcell"] { background: var(--role-working) !important; }` });
        }
        await expect(key(page, 5)).toHaveCSS("background-color", theme === "light" ? "rgb(239, 241, 245)" : "rgb(16, 16, 16)");
        await light(page, 1, "green");
        await light(page, 5, "yellow");
        for (const n of [1, 5]) {
          const badge = key(page, n).locator("span").first();
          await expect(badge).toHaveCSS("color", "rgb(14, 14, 20)");
          await expect(badge.locator("[data-key-mark]")).toBeVisible();
          const contrast = await badge.evaluate(el => {
            const css = getComputedStyle(el);
            const luminance = (rgb: string) => {
              const values = rgb.match(/\d+/g)!.slice(0, 3).map(v => {
                const c = Number(v) / 255;
                return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
              });
              return values[0]! * 0.2126 + values[1]! * 0.7152 + values[2]! * 0.0722;
            };
            return (luminance(css.backgroundColor) + 0.05) / (luminance(css.color) + 0.05);
          });
          expect(contrast).toBeGreaterThan(7);
          const box = await badge.boundingBox();
          const cell = await key(page, n).boundingBox();
          expect(box).not.toBeNull();
          expect(box!.x).toBeGreaterThanOrEqual(cell!.x);
          expect(box!.x + box!.width).toBeLessThanOrEqual(Math.min(width, cell!.x + cell!.width));
        }
        await page.screenshot({ path: info.outputPath(`keys-${theme}.png`) });
      }
    });
  }
}

async function preferences(page: Page) {
  await page.getByRole("button", { name: "Command palette", exact: true }).click();
  await page.getByRole("combobox", { name: "Search actions" }).fill("preferences");
  await page.keyboard.press("Enter");
  return page.getByRole("dialog", { name: "Preferences" });
}

test("review: default sorting follows displayed notation and an explicit choice wins", async ({ page }) => {
  await open(page);
  const codes = () => page.getByRole("tabpanel").locator('[role="gridcell"][data-col="key"] [data-key-text]').allTextContents();
  await page.getByRole("columnheader").filter({ hasText: /^key$/i }).click();
  await expect.poll(codes).toEqual(["1A", "6A", "7A", "8A", "8B", "9A", "9B", "10A", "11A", "12A", "Unknown", ""]);
  const dialog = await preferences(page);
  await dialog.getByRole("radiogroup", { name: "Key display format", exact: true }).getByRole("radio", { name: "Classic", exact: true }).click();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect.poll(codes).toEqual(["Abm", "Am", "Bm", "C", "Dbm", "Dm", "Em", "F#m", "G", "Gm", "Unknown", ""]);
  const again = await preferences(page);
  await again.getByRole("radio", { name: "Musically", exact: false }).click();
  await again.getByRole("button", { name: "Close", exact: true }).click();
  await expect.poll(codes).toEqual(["Abm", "Gm", "Dm", "Am", "C", "Em", "G", "Bm", "F#m", "Dbm", "Unknown", ""]);
});

test("review: full existing Classic preferences stay Classic", async ({ page }) => {
  await open(page, false, { keyDisplay: "classic", keySort: "alphabetical" });
  await expect(key(page, 1).locator("[data-key-text]")).toHaveText("Am");
  await page.reload();
  await expect(key(page, 1).locator("[data-key-text]")).toHaveText("Am");
});

test("review: Intake queue uses the same notation as the loaded player", async ({ page }) => {
  await open(page);
  await page.getByRole("tab", { name: /^Intake/ }).click();
  const first = page.getByRole("listbox", { name: "Intake queue" }).getByRole("option").first();
  await first.click();
  const displayed = await player(page).getByTestId("compact-player-key").innerText();
  expect(displayed).toMatch(/^\d+[AB]$/);
  await expect(first).toContainText(displayed);
  const dialog = await preferences(page);
  await dialog.getByRole("radio", { name: "Classic", exact: true }).click();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  const classic = await player(page).getByTestId("compact-player-key").innerText();
  await expect(first).toContainText(classic);
});

test("review: lights have visible symbols and accessible names; shell has no redundant source selector", async ({ page }) => {
  await open(page);
  await load(page, 1);
  const match = key(page, 1).getByRole("img", { name: "8A: Harmonic match" });
  const boost = key(page, 5).getByRole("img", { name: "10A: Energy boost (+2)", exact: true });
  await expect(match).toBeVisible();
  await expect(boost).toBeVisible();
  await expect(match.locator("[data-key-mark]")).toHaveText("=");
  await expect(boost.locator("[data-key-mark]")).toHaveText("↑");
  await expect(page.getByRole("button", { name: "Traffic Light deck", exact: true })).toHaveCount(0);
});

test("review: shell omits the legacy traffic source selector", async ({ page }) => {
  await open(page);
  await expect(page.getByRole("button", { name: "Traffic Light deck", exact: true })).toHaveCount(0);
});

test("a fresh install gets Camelot without a seeded preferences object", async ({ page }) => {
  await page.addInitScript(() => localStorage.removeItem("rbl.preferences"));
  await page.goto("/?keyLights=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await page.getByRole("button", { name: "Command palette", exact: true }).click();
  await page.getByRole("combobox", { name: "Search actions" }).fill("Use the view shell");
  await page.keyboard.press("Enter");
  await page.getByRole("treeitem", { name: "All Tracks", exact: true }).click();
  await expect(key(page, 1).locator("[data-key-text]")).toHaveText("8A");
});

test("stored notation is separate, keyboard operable, and survives a display change", async ({ page }) => {
  await open(page);
  await load(page, 1);
  const dialog = await preferences(page);
  await dialog.getByRole("tab", { name: "Analysis", exact: true }).click();
  const stored = dialog.getByRole("combobox", { name: "Stored key notation", exact: true });
  await expect(stored).toHaveValue("classic");
  await stored.focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await expect(stored).toHaveValue("alphanumeric");
  await dialog.getByRole("tab", { name: "View", exact: true }).click();
  await dialog.getByRole("radio", { name: "Classic", exact: true }).click();
  const automatic = dialog.getByRole("radio", { name: "Follow displayed notation", exact: true });
  await automatic.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Enter");
  await expect(automatic).toBeChecked();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(playButton(page)).toBeVisible();
  await expect(key(page, 1).locator("[data-key-text]")).toHaveText("Am");
  await page.reload();
  const again = await preferences(page);
  await again.getByRole("tab", { name: "Analysis", exact: true }).click();
  await expect(again.getByRole("combobox", { name: "Stored key notation", exact: true })).toHaveValue("alphanumeric");
});
