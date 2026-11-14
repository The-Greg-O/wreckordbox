/**
 * The view shell's top bar at its narrowest (wbx-45.22): at 436, 400 and
 * 360 px wide and 0.8, 1, 1.25, in a dark and a light theme, the four tabs,
 * the waveform theme chooser and the palette opener are all drawn, inside
 * the window and the top bar, none over another and none over the player;
 * each is worked by pointer and reached by keyboard. Bounds, not text.
 *
 * The mock's Intake count (1701) makes its tab wide, as a real backlog does.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { audit, shortfalls } from "./lightAudit";
import { open } from "./compactPlayer";
import { BROWSER_THEME_EVENT, type ThemeRoles } from "../src/lib/themeRoles";

const LATTE: ThemeRoles = {
  mode: "light", ground: "#d7d8dc", panel: "#e3e4e8", working: "#eff1f5", hairline: "#dce0e8",
  selection: "#ccd0da", border: "#acb0be", text: "#4c4f69", textDim: "#9ca0b0", textBright: "#4c4f69",
  accent: "#1e66f5", onAccent: "#ffffff", tabFrom: "#ea76cb", tabTo: "#df8e1d",
};
const SCALES = [{ stop: 2, factor: 0.8 }, { stop: 4, factor: 1 }, { stop: 6, factor: 1.25 }];
const HEIGHT = 700;

const topBar = (page: Page) => page.getByTestId("shell-top-bar");
const tabs = (page: Page) => page.getByRole("tablist", { name: "Views" }).getByRole("tab");
const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const chooser = (page: Page) => topBar(page).getByRole("button", { name: /^Waveform color/ });
const opener = (page: Page) => topBar(page).getByRole("button", { name: "Command palette" });
const menu = (page: Page) => page.getByRole("menu", { name: "Waveform color" });
const palette = (page: Page) => page.getByRole("dialog", { name: "Command palette" });

type Box = { x: number; y: number; width: number; height: number };
const box = async (l: Locator): Promise<Box> => (await l.boundingBox())!;
const within = (a: Box, b: Box) => a.x >= b.x - 0.5 && a.y >= b.y - 0.5 && a.x + a.width <= b.x + b.width + 0.5 && a.y + a.height <= b.y + b.height + 0.5;
const overlap = (a: Box, b: Box) => Math.max(0, Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x)) * Math.max(0, Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y));

async function controls(page: Page): Promise<{ what: string; at: Box; element: Locator }[]> {
  const found: { what: string; at: Box; element: Locator }[] = [];
  for (const [i, t] of (await tabs(page).all()).entries()) found.push({ what: `tab ${i}`, at: await box(t), element: t });
  found.push({ what: "chooser", at: await box(chooser(page)), element: chooser(page) });
  found.push({ what: "opener", at: await box(opener(page)), element: opener(page) });
  return found;
}

/** Every control drawn whole, inside the window and the bar, none over another, the bar above the player. */
async function expectLaidOut(page: Page, width: number): Promise<void> {
  await expect(tabs(page)).toHaveCount(4);
  await expect(chooser(page)).toBeVisible();
  await expect(opener(page)).toBeVisible();
  const bar = await box(topBar(page));
  const window = { x: 0, y: 0, width, height: HEIGHT };
  const all = await controls(page);
  for (const c of all) {
    expect(within(c.at, window), `${c.what} in the window`).toBe(true);
    expect(within(c.at, bar), `${c.what} in the top bar`).toBe(true);
    // What it draws is drawn whole, not cut.
    expect(await c.element.evaluate((e) => e.scrollWidth <= e.clientWidth + 0.5), `${c.what} whole`).toBe(true);
  }
  for (const [i, a] of all.entries()) {
    for (const b of all.slice(i + 1)) expect(overlap(a.at, b.at), `${a.what} over ${b.what}`).toBeLessThan(0.5);
  }
  const player = await box(page.getByTestId("shell-player"));
  expect(bar.y + bar.height).toBeLessThanOrEqual(player.y + 0.5);
}

async function announce(page: Page, roles: ThemeRoles | null): Promise<void> {
  await page.evaluate(([name, detail]) => void window.dispatchEvent(new CustomEvent(name as string, { detail })), [BROWSER_THEME_EVENT, { roles, font: null }] as const);
}

for (const width of [436, 400, 360]) {
  for (const scale of SCALES) {
    for (const theme of ["dark", "light"] as const) {
      test(`${width}px at ${Math.round(scale.factor * 100)}%, ${theme}: every control is drawn, apart, and works by pointer and keyboard`, async ({ page }) => {
        await page.setViewportSize({ width, height: HEIGHT });
        await open(page, "", { uiScale: scale.stop });
        if (theme === "light") {
          await announce(page, LATTE);
          await expect(page.locator("html")).toHaveAttribute("data-theme-mode", "light");
        }
        await expectLaidOut(page, width);
        if (theme === "light") expect([...new Set(shortfalls((await audit(topBar(page))).seen))]).toEqual([]);

        // Pointer: each tab switches the view; the chooser opens its menu (inside the window) and chooses; the opener opens the palette.
        for (const name of ["Review", "Library"]) {
          await tab(page, name).click();
          await expect(tab(page, name)).toHaveAttribute("aria-selected", "true");
        }
        await chooser(page).click();
        await expect(menu(page)).toBeVisible();
        expect(within(await box(menu(page)), { x: 0, y: 0, width, height: HEIGHT })).toBe(true);
        await menu(page).getByRole("menuitemradio", { name: "RGB", exact: true }).click();
        await expect(page.locator("html")).toHaveAttribute("data-wave-theme", "rgb");
        await opener(page).click();
        await expect(palette(page)).toBeVisible();
        await page.keyboard.press("Escape");
        await expect(palette(page)).toBeHidden();

        // Keyboard: Tab from the first tab reaches the chooser, then the opener, and each answers Enter.
        await tab(page, "Library").focus();
        const reached: string[] = [];
        for (let i = 0; i < 6; i++) {
          await page.keyboard.press("Tab");
          reached.push(await page.evaluate(() => document.activeElement?.getAttribute("aria-label") ?? document.activeElement?.textContent ?? ""));
          if (reached.at(-1) === "Command palette") break;
        }
        expect(reached.some((r) => r.startsWith("Waveform color"))).toBe(true);
        expect(reached.at(-1)).toBe("Command palette");
        await page.keyboard.press("Enter");
        await expect(palette(page)).toBeVisible();
        await page.keyboard.press("Escape");
        await chooser(page).focus();
        await page.keyboard.press("Enter");
        await expect(menu(page)).toBeVisible();
        await page.keyboard.press("Escape");
        await expect(menu(page)).toBeHidden();
        await expectLaidOut(page, width);
      });
    }
  }
}

test("wider windows keep the bar as it was: one row, the opener's word and the chooser's name", async ({ page }) => {
  for (const width of [1280, 640]) {
    await page.setViewportSize({ width, height: HEIGHT });
    await open(page);
    const bar = await box(topBar(page));
    const tabBox = await box(tab(page, "Library"));
    // One row: the opener and the chooser on the tabs' line.
    expect(Math.abs((await box(opener(page))).y + (await box(opener(page))).height / 2 - (tabBox.y + tabBox.height / 2))).toBeLessThan(2);
    expect(bar.height).toBeLessThan(tabBox.height + 4);
    await expect(opener(page)).toContainText("Commands");
  }
  await expect(chooser(page).locator("[data-wave-name]")).toBeHidden(); // 640: the glyph band (wbx-45.9)
});
