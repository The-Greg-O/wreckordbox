/**
 * Waveform colour themes and sharp waveforms (ticket #48) across every
 * waveform the window has: upstream's deck (overview, detail, the rows'
 * previews), the view shell's compact player overview, and Intake's large
 * player detail.
 *
 * The theme the app shell would send is stood in for with the
 * `wreckordbox:theme` event (`themeRoles.ts`): `wave` is the Omarchy
 * palette's three colours for the Theme waveform, `waveform` the theme
 * file's choice, `roles` the window's chrome.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { player as shellPlayer, seedShell } from "./compactPlayer";

const PINK = [243, 139, 168] as const;
const MEASURED_BLUE = [0, 85, 225] as const;
const WAVE = { low: "#f38ba8", mid: "#fab387", high: "#f9e2af" };

/** Every role of the window's chrome, all in one loud colour. */
const roles = (colour: string) => ({
  mode: "dark", ground: "#000000", panel: "#111111", working: "#222222", hairline: "#333333",
  selection: "#444444", border: "#555555", text: "#dddddd", textDim: "#999999", textBright: colour,
  accent: colour, onAccent: "#000000", tabFrom: colour, tabTo: colour,
});

const showTheme = (page: Page, theme: Record<string, unknown>) =>
  page.evaluate((detail) => window.dispatchEvent(new CustomEvent("wreckordbox:theme", { detail })), { roles: null, font: null, ...theme });

/** Whether a canvas has a pixel of exactly this colour, anywhere. */
const paints = (canvas: Locator, rgb: readonly [number, number, number]) =>
  canvas.evaluate((c: HTMLCanvasElement, [r, g, b]) => {
    const ctx = c.getContext("2d");
    if (!ctx || c.width === 0 || c.height === 0) return false;
    const data = ctx.getImageData(0, 0, c.width, c.height).data;
    for (let i = 0; i < data.length; i += 4) {
      if (data[i] === r && data[i + 1] === g && data[i + 2] === b && data[i + 3] === 255) return true;
    }
    return false;
  }, rgb);

const pixels = (canvas: Locator) => canvas.evaluate((c: HTMLCanvasElement) =>
  Array.from(c.getContext("2d")?.getImageData(0, 0, c.width, c.height).data ?? []).join(","));

const deck = (page: Page) => page.getByRole("region", { name: "Preview player" });
const deckOverview = (page: Page) => page.getByTestId("player-overview").locator("canvas").first();

async function openDeck(page: Page, view: Record<string, unknown> = {}) {
  await page.addInitScript((extra) => {
    if (sessionStorage.getItem("wave-seeded")) return;
    sessionStorage.setItem("wave-seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify({ advanced: { protectLibrary: false }, view: extra }));
  }, view);
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await page.locator('[role="gridcell"][data-col="title"]').nth(3).dblclick();
  await expect(deck(page).getByRole("button", { name: "Play", exact: true })).toBeEnabled();
}

async function waveformChoices(page: Page) {
  await page.getByRole("banner").getByRole("button", { name: "Settings" }).click();
  const dialog = page.getByRole("dialog", { name: "Preferences" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("tab", { name: "Color" }).click();
  return { dialog, group: dialog.getByRole("radiogroup", { name: "Waveform color" }) };
}

test("the theme file's waveform theme wins over Preferences, which shows it greyed; without it the preference is back", async ({ page }) => {
  await openDeck(page, { waveformColor: "rgb" });
  const overview = deckOverview(page);
  await expect.poll(() => pixels(overview)).not.toBe("");
  const rgb = await pixels(overview);

  await showTheme(page, { waveform: "theme", wave: WAVE });
  await expect.poll(() => paints(overview, PINK)).toBe(true);
  let { dialog, group } = await waveformChoices(page);
  await expect(group.getByRole("radio", { name: "Theme", exact: true })).toBeChecked();
  for (const name of ["BLUE", "RGB", "3Band", "Theme"]) {
    await expect(group.getByRole("radio", { name, exact: true })).toBeDisabled();
  }
  await expect(dialog.getByText("Set by the theme file (waveform).")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();

  // The file no longer names one: the DJ's own choice, untouched, is back.
  await showTheme(page, { wave: WAVE });
  await expect.poll(() => pixels(overview)).toBe(rgb);
  ({ dialog, group } = await waveformChoices(page));
  await expect(group.getByRole("radio", { name: "RGB", exact: true })).toBeChecked();
  await expect(group.getByRole("radio", { name: "RGB", exact: true })).toBeEnabled();
  await expect(dialog.getByText("Set by the theme file (waveform).")).toHaveCount(0);
});

test("the Theme waveform takes the Omarchy palette's colours, never the window's roles", async ({ page }) => {
  await openDeck(page, { waveformColor: "theme" });
  const overview = deckOverview(page);
  // No Omarchy theme: Tokyo Night's blue.
  await expect.poll(() => paints(overview, [122, 162, 247])).toBe(true);

  await showTheme(page, { roles: roles("#00ff00"), wave: WAVE });
  await expect.poll(() => paints(overview, PINK)).toBe(true);
  const drawn = await pixels(overview);
  // Another chrome (a theme file's colours, say), the same Omarchy palette: the same waveform.
  await showTheme(page, { roles: roles("#ff00ff"), wave: WAVE });
  await page.waitForTimeout(200);
  expect(await pixels(overview)).toBe(drawn);
  // 3Band never moves with any of it.
  await showTheme(page, { roles: roles("#ff00ff"), wave: WAVE, waveform: "3band" });
  await expect.poll(() => paints(overview, MEASURED_BLUE)).toBe(true);
  expect(await paints(overview, PINK)).toBe(false);
});

test("the compact player's overview and Intake's large detail draw in the waveform theme in force", async ({ page }) => {
  await seedShell(page);
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  await page.locator('[role="gridcell"][data-col="title"]').nth(3).dblclick();
  const compact = shellPlayer(page).getByTestId("compact-player-overview").locator("canvas").first();
  await expect.poll(() => paints(compact, MEASURED_BLUE)).toBe(true);

  await showTheme(page, { waveform: "theme", wave: WAVE });
  await expect.poll(() => paints(compact, PINK)).toBe(true);
  expect(await paints(compact, MEASURED_BLUE)).toBe(false);

  await page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "Intake" }).click();
  const queued = page.getByRole("listbox", { name: "Intake queue" }).getByRole("option").filter({ hasText: /· \d{2,3} ·/ }).first();
  await queued.click();
  const large = shellPlayer(page).getByTestId("large-player-wave").locator("canvas").first();
  await expect.poll(() => paints(large, PINK)).toBe(true);
  await expect.poll(() => paints(shellPlayer(page).getByTestId("compact-player-overview").locator("canvas").first(), PINK)).toBe(true);

  await showTheme(page, { waveform: "blue" });
  await expect.poll(() => paints(large, PINK)).toBe(false);
});

/**
 * Each waveform canvas's store against the device pixels its box covers:
 * equal means one store pixel per device pixel, never stretched or blended.
 */
const mismatches = (page: Page, selector: string) => page.evaluate((sel) => {
  const out: string[] = [];
  const canvases = [...document.querySelectorAll<HTMLCanvasElement>(sel)].filter((c) => c.width > 0);
  for (const c of canvases.slice(0, 6)) {
    const box = c.getBoundingClientRect();
    const w = box.width * devicePixelRatio;
    const h = box.height * devicePixelRatio;
    if (Math.abs(w - c.width) > 0.01 || Math.abs(h - c.height) > 0.01) out.push(`${c.width}x${c.height} over ${w}x${h}`);
  }
  return { count: canvases.length, out };
}, selector);

for (const ratio of [1, 2]) {
  test.describe(`at a ${ratio}x display`, () => {
    test.use({ deviceScaleFactor: ratio });
    // The smallest and the largest interface scale, and a fractional one between.
    for (const step of [0, 6, 7]) {
      test(`every waveform matches device pixels at interface scale step ${step}`, async ({ page }) => {
        await openDeck(page, { uiScale: step });
        for (const [where, selector] of [
          ["overview", '[data-testid="player-overview"] canvas'],
          ["detail", '[data-testid="player-detail"] canvas'],
          ["row previews", '[role="gridcell"] canvas'],
        ] as const) {
          await expect.poll(async () => (await mismatches(page, selector)).count, where).toBeGreaterThan(0);
          expect((await mismatches(page, selector)).out, where).toEqual([]);
        }

        await page.evaluate(() => sessionStorage.removeItem("wave-seeded"));
        await page.evaluate((s) => {
          localStorage.setItem("rbl.preferences", JSON.stringify({ advanced: { protectLibrary: false }, view: { viewShell: true, uiScale: s } }));
          sessionStorage.setItem("wave-seeded", "1");
        }, step);
        await page.reload();
        await expect(page.getByTestId("shell")).toBeVisible();
        await page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "Intake" }).click();
        await page.getByRole("listbox", { name: "Intake queue" }).getByRole("option").filter({ hasText: /· \d{2,3} ·/ }).first().click();
        for (const [where, selector] of [
          ["compact overview", '[data-testid="compact-player-overview"] canvas'],
          ["large detail", '[data-testid="large-player-wave"] canvas'],
        ] as const) {
          await expect.poll(async () => (await mismatches(page, selector)).count, where).toBeGreaterThan(0);
          expect((await mismatches(page, selector)).out, where).toEqual([]);
        }
      });
    }
  });
}
