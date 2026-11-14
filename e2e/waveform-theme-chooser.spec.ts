/**
 * The waveform theme chooser in the view shell's top bar (wbx-45.9;
 * docs/design/screens/intake/README.md: 3Band, RGB, Blue, Theme "chosen
 * from the top bar"; the mock's "wave 3Band ▾").
 *
 * What is checked is the effect: the theme every waveform draws
 * (`<html data-wave-theme>`, set by the window from `useWaveformTheme`) and
 * the stored preference. A theme file that names a waveform wins, and the
 * chooser is greyed with the reason Preferences gives. It is worked by
 * pointer and keyboard in every view, owns Space and Enter (the deck does
 * not start), stays inside the top bar at 1280, 900, 640 and 436 px and
 * 0.8, 1, 1.25 (giving way at 436 and 125%, where the bar is already full),
 * and reads under a light theme.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { audit, shortfalls } from "./lightAudit";
import { load, open, pauseButton, playButton } from "./compactPlayer";
import { BROWSER_THEME_EVENT, type Theme, type ThemeRoles } from "../src/lib/themeRoles";

const LATTE: ThemeRoles = {
  mode: "light", ground: "#d7d8dc", panel: "#e3e4e8", working: "#eff1f5", hairline: "#dce0e8",
  selection: "#ccd0da", border: "#acb0be", text: "#4c4f69", textDim: "#9ca0b0", textBright: "#4c4f69",
  accent: "#1e66f5", onAccent: "#ffffff", tabFrom: "#ea76cb", tabTo: "#df8e1d",
};
const WHITE: ThemeRoles = {
  mode: "light", ground: "#e8e8e8", panel: "#f5f5f5", working: "#ffffff", hairline: "#c0c0c0",
  selection: "#c0c0c0", border: "#808080", text: "#000000", textDim: "#c0c0c0", textBright: "#000000",
  accent: "#6e6e6e", onAccent: "#ffffff", tabFrom: "#2e2e2e", tabTo: "#4a4a4a",
};

const VIEWS = ["Library", "Intake", "Review", "Stick"] as const;
const LABEL = { "3band": "3Band", rgb: "RGB", blue: "BLUE", theme: "Theme" } as const;
type Wave = keyof typeof LABEL;
const REASON = "Set by the theme file (waveform).";

const topBar = (page: Page) => page.getByTestId("shell-top-bar");
const chooser = (page: Page) => topBar(page).getByRole("button", { name: /^Waveform color/ });
const menu = (page: Page) => page.getByRole("menu", { name: "Waveform color" });
const choice = (page: Page, wave: Wave) => menu(page).getByRole("menuitemradio", { name: LABEL[wave], exact: true });
const drawn = (page: Page) => page.locator("html").getAttribute("data-wave-theme");
const stored = (page: Page) => page.evaluate(() => (JSON.parse(localStorage.getItem("rbl.preferences") ?? "{}") as { view?: { waveformColor?: string } }).view?.waveformColor);

async function announce(page: Page, theme: Theme): Promise<void> {
  await page.evaluate(([name, detail]) => void window.dispatchEvent(new CustomEvent(name as string, { detail })), [BROWSER_THEME_EVENT, theme] as const);
}

async function showView(page: Page, view: (typeof VIEWS)[number]): Promise<void> {
  const tab = page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: view });
  await tab.click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
}

async function expectInForce(page: Page, wave: Wave): Promise<void> {
  await expect(chooser(page)).toHaveAccessibleName(`Waveform color: ${LABEL[wave]}`);
  // Its name is drawn where the top bar has room for it (see the bounds tests).
  if (await chooser(page).locator("[data-wave-name]").isVisible()) await expect(chooser(page)).toContainText(LABEL[wave]);
  await expect.poll(() => drawn(page)).toBe(wave);
}

test("in every view the top bar shows the waveform theme in force and chooses among the four by pointer", async ({ page }) => {
  await open(page);
  await expectInForce(page, "3band");
  const next: Wave[] = ["rgb", "blue", "theme", "3band"];
  for (const [at, view] of VIEWS.entries()) {
    await showView(page, view);
    const wave = next[at]!;
    await chooser(page).click();
    await expect(menu(page)).toBeVisible();
    // The four, the one in force ticked.
    await expect(menu(page).getByRole("menuitemradio")).toHaveCount(4);
    await expect(menu(page).getByRole("menuitemradio", { checked: true })).toHaveCount(1);
    await choice(page, wave).click();
    await expect(menu(page)).toBeHidden();
    await expectInForce(page, wave);
    expect(await stored(page)).toBe(wave);
  }
  // Remembered: the next start draws the last choice.
  await chooser(page).click();
  await choice(page, "theme").click();
  await page.reload();
  await expectInForce(page, "theme");
});

test("by keyboard: it owns Space and Enter, arrows move, Escape leaves the choice alone", async ({ page }) => {
  await load(page);
  await expect(playButton(page)).toBeVisible();
  await chooser(page).focus();
  // Space opens the menu and does not reach the deck.
  await page.keyboard.press("Space");
  await expect(menu(page)).toBeVisible();
  await expect(choice(page, "3band")).toBeFocused();
  await expect(pauseButton(page)).toHaveCount(0);
  await page.keyboard.press("ArrowDown");
  await expect(choice(page, "rgb")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(menu(page)).toBeHidden();
  await expectInForce(page, "rgb");
  await expect(chooser(page)).toBeFocused();
  await expect(pauseButton(page)).toHaveCount(0);

  // Enter opens it on the one in force; Up wraps; Escape closes with nothing changed.
  await page.keyboard.press("Enter");
  await expect(choice(page, "rgb")).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(choice(page, "3band")).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(choice(page, "theme")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(menu(page)).toBeHidden();
  await expect(chooser(page)).toBeFocused();
  await expectInForce(page, "rgb");
  expect(await stored(page)).toBe("rgb");
  await expect(pauseButton(page)).toHaveCount(0);
});

test("while the theme file sets the waveform it is shown greyed with the reason, and choosing does nothing", async ({ page }) => {
  await open(page);
  await announce(page, { roles: null, font: null, waveform: "blue" });
  await expectInForce(page, "blue");
  await expect(chooser(page)).toHaveAttribute("aria-disabled", "true");
  await expect(chooser(page)).toHaveAccessibleDescription(REASON);
  for (const view of VIEWS) {
    await showView(page, view);
    await expect(chooser(page)).toHaveAttribute("aria-disabled", "true");
  }
  // Opened (it is greyed, not dead: a pointer still opens it), it says why, and its entries cannot be chosen.
  await chooser(page).click({ force: true });
  await expect(menu(page)).toBeVisible();
  await expect(menu(page)).toContainText(REASON);
  for (const wave of Object.keys(LABEL) as Wave[]) await expect(choice(page, wave)).toBeDisabled();
  await expect(choice(page, "blue")).toHaveAttribute("aria-checked", "true");
  await choice(page, "rgb").click({ force: true });
  await expectInForce(page, "blue");
  expect(await stored(page) ?? "3band").toBe("3band");
  await page.keyboard.press("Escape");
  await expect(menu(page)).toBeHidden();

  // The file lets go: the DJ's own choice is back, and live.
  await announce(page, { roles: null, font: null });
  await expectInForce(page, "3band");
  await expect(chooser(page)).not.toHaveAttribute("aria-disabled", "true");
  await chooser(page).click();
  await choice(page, "rgb").click();
  await expectInForce(page, "rgb");
});

type Box = { x: number; y: number; width: number; height: number };
const box = async (l: Locator): Promise<Box> => (await l.boundingBox())!;
const within = (a: Box, b: Box) => a.x >= b.x - 0.5 && a.y >= b.y - 0.5 && a.x + a.width <= b.x + b.width + 0.5 && a.y + a.height <= b.y + b.height + 0.5;
const apart = (a: Box, b: Box) => a.x + a.width <= b.x + 0.5 || b.x + b.width <= a.x + 0.5;

for (const width of [1280, 900, 640, 436]) {
  for (const scale of [{ stop: 2, factor: 0.8 }, { stop: 4, factor: 1 }, { stop: 6, factor: 1.25 }]) {
    test(`${width}px at ${Math.round(scale.factor * 100)}%: inside the top bar, clear of the tabs and the palette opener, its menu inside the window`, async ({ page }) => {
      await page.setViewportSize({ width, height: 800 });
      await open(page, "", { uiScale: scale.stop });
      const opener = topBar(page).getByRole("button", { name: "Command palette" });
      const layout = width / scale.factor;
      if (layout <= 400) {
        // 436 at 125%: the tabs and the opener already fill the bar (follow-up bead); the chooser gives way
        // and leaves the opener where it was, inside the window.
        await expect(chooser(page)).toBeHidden();
        const o = await box(opener);
        expect(o.x + o.width).toBeLessThanOrEqual(width + 0.5);
        return;
      }
      await expect(chooser(page)).toBeVisible();
      const bar = await box(topBar(page));
      const c = await box(chooser(page));
      expect(within(c, bar)).toBe(true);
      expect(c.x + c.width).toBeLessThanOrEqual(width + 0.5);
      // What it draws is drawn whole, not cut.
      expect(await chooser(page).evaluate((e) => e.scrollWidth <= e.clientWidth + 0.5)).toBe(true);
      const o = await box(opener);
      expect(apart(o, c)).toBe(true);
      expect(o.x + o.width).toBeLessThanOrEqual(width + 0.5);
      for (const tab of await page.getByRole("tablist", { name: "Views" }).getByRole("tab").all()) expect(apart(await box(tab), c)).toBe(true);
      // Where it has room its name is drawn whole; elsewhere the glyph stands in.
      const named = layout > 720 || layout <= 600;
      await expect(chooser(page).locator("[data-wave-name]")).toBeVisible({ visible: named });

      await chooser(page).click();
      const m = await box(menu(page));
      expect(within(m, { x: 0, y: 0, width, height: 800 })).toBe(true);
      for (const wave of Object.keys(LABEL) as Wave[]) {
        const item = await box(choice(page, wave));
        expect(within(item, m)).toBe(true);
      }
    });
  }
}

test("under a light theme the chooser and its menu can be read", async ({ page }) => {
  await open(page);
  for (const roles of [LATTE, WHITE]) {
    await announce(page, { roles, font: null });
    await expect(page.locator("html")).toHaveAttribute("data-theme-mode", "light");
    const closed = await audit(chooser(page));
    expect(closed.seen.length).toBeGreaterThan(0);
    expect([...new Set(shortfalls(closed.seen))], `${roles.working} chooser`).toEqual([]);
    await chooser(page).click();
    const opened = await audit(menu(page));
    expect(opened.seen.length).toBeGreaterThan(3);
    expect([...new Set(shortfalls(opened.seen))], `${roles.working} menu`).toEqual([]);
    await page.keyboard.press("Escape");
    // Greyed by the theme file, still readable.
    await announce(page, { roles, font: null, waveform: "rgb" });
    await expect(chooser(page)).toHaveAttribute("aria-disabled", "true");
    await chooser(page).click({ force: true });
    const greyed = await audit(menu(page));
    expect([...new Set(shortfalls(greyed.seen))], `${roles.working} greyed menu`).toEqual([]);
    await page.keyboard.press("Escape");
  }
});
