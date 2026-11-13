/**
 * The Library at half and narrow widths (wbx-60; docs/design/tokens.md
 * "Breakpoints", docs/design/screens/library "Sizes"). By the window's own
 * width in CSS pixels, so the interface scale counts:
 *
 * - above 800: the tree, the chosen columns (the title giving room first);
 *   with the second list shown the main list drops date added and narrows
 *   MyTags, and anything that still does not fit gives way in a fixed order;
 * - 800 and below: no tree, no second list; title, BPM, key, Energy, time;
 * - 520 and below (about 436): title, BPM, key and Energy only.
 *
 * The Track Filter bar and the key lights stay at every width. Nothing
 * overlaps, nothing is cut off, nothing scrolls sideways, and every control
 * of the bar can take the keyboard's focus. The old layout is unchanged.
 *
 * Bounds, not text. Mock: `?keyLights=1` (rows "Key light 01".. with keys
 * that light against the loaded first one), `?filterFixture=1`.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { player } from "./compactPlayer";
import { DEFAULT_PREFERENCES } from "../src/lib/preferences";

const SCALES = [{ stop: 2, factor: 0.8 }, { stop: 4, factor: 1 }, { stop: 6, factor: 1.25 }];
const WIDTHS = [1280, 900, 640, 436];

const working = (page: Page) => page.getByRole("tabpanel");
const scroller = (page: Page) => working(page).getByTestId("track-scroll");
const rows = (page: Page) => scroller(page).locator('[role="row"][aria-selected]');
const bar = (page: Page) => page.getByRole("search", { name: "Track Filter" });
const tree = (page: Page) => page.getByTestId("shell-tree");
const target = (page: Page) => page.getByRole("region", { name: "Second list" });

async function open(page: Page, width: number, stop: number, query: string, viewShell = true): Promise<void> {
  await page.setViewportSize({ width, height: 800 });
  await page.addInitScript((full) => {
    if (sessionStorage.getItem("narrow-seeded")) return;
    sessionStorage.setItem("narrow-seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify(full));
  }, { ...DEFAULT_PREFERENCES, advanced: { ...DEFAULT_PREFERENCES.advanced, protectLibrary: false },
    view: { ...DEFAULT_PREFERENCES.view, viewShell, uiScale: stop } });
  await page.goto(`/${query}`);
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
}

/** Opens All Tracks from the tree, at full width when the tree is not drawn at this one. */
async function allTracks(page: Page, width: number): Promise<void> {
  if (!(await tree(page).isVisible())) {
    await page.setViewportSize({ width: 1600, height: 800 });
  }
  await tree(page).getByRole("treeitem", { name: /^All Tracks/ }).click();
  await expect(page.getByTestId("browser-title")).toContainText("All Tracks");
  await page.setViewportSize({ width, height: 800 });
  await expect(rows(page).first()).toBeVisible();
}

/** The columns the first row draws, left to right. */
async function drawn(page: Page): Promise<string[]> {
  return rows(page).first().locator("[data-col]").evaluateAll((cells) => cells
    .filter((cell) => cell.getClientRects().length > 0)
    .map((cell) => cell.getAttribute("data-col")!));
}

type Box = { x: number; y: number; width: number; height: number; name: string };
const overlaps = (a: Box, b: Box) =>
  a.x < b.x + b.width - 0.5 && b.x < a.x + a.width - 0.5 && a.y < b.y + b.height - 0.5 && b.y < a.y + a.height - 0.5;

/** Every visible match of `selector` lies across within [left, right] and none covers another (nesting aside). */
async function expectLaidOut(scope: Locator, selector: string, left: number, right: number): Promise<void> {
  const { boxes, nested } = await scope.first().evaluate((root, sel) => {
    const shown = [...root.querySelectorAll(sel)].filter((el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden");
    return {
      boxes: shown.map((el) => { const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height, name: `${el.tagName} ${el.getAttribute("aria-label") ?? el.getAttribute("data-col") ?? el.textContent?.trim().slice(0, 30) ?? ""}` }; }),
      nested: shown.flatMap((a, i) => shown.flatMap((b, j) => (i !== j && a.contains(b) ? [`${Math.min(i, j)}:${Math.max(i, j)}`] : []))),
    };
  }, selector);
  expect(boxes.length, selector).toBeGreaterThan(0);
  const inside = new Set(nested);
  for (const box of boxes) {
    expect(box.x, box.name).toBeGreaterThanOrEqual(left - 0.5);
    expect(box.x + box.width, box.name).toBeLessThanOrEqual(right + 0.5);
  }
  for (let i = 0; i < boxes.length; i++) {
    for (let j = i + 1; j < boxes.length; j++) {
      if (inside.has(`${i}:${j}`)) continue;
      expect(overlaps(boxes[i]!, boxes[j]!), `${boxes[i]!.name} over ${boxes[j]!.name}`).toBe(false);
    }
  }
}

/** The list does not scroll sideways: what it draws fits its width. */
async function expectNoSidewaysScroll(page: Page): Promise<void> {
  const [scrollWidth, clientWidth] = await scroller(page).evaluate((el) => [el.scrollWidth, el.clientWidth]);
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 1);
}

/** The window's width as the layout sees it: the interface scale counts. */
const layoutWidth = (width: number, factor: number) => width / factor;

for (const width of WIDTHS) {
  for (const scale of SCALES) {
    test(`${width}px at ${Math.round(scale.factor * 100)}%: what remains fits, and the filter bar and key lights stay`, async ({ page }) => {
      await open(page, width, scale.stop, "?writable=1&keyLights=1");
      await allTracks(page, width);
      const lw = layoutWidth(width, scale.factor);
      // Load the first key-light row: the others' keys light against it.
      const loaded = rows(page).filter({ has: page.locator('[data-col="title"]', { hasText: /^Key light 01$/ }) });
      await loaded.locator('[data-col="title"]').dblclick();
      await expect(player(page).getByTestId("compact-player-title")).toHaveText("Key light 01");

      // The tree gives way at 800 and below.
      if (lw <= 800) await expect(tree(page)).toBeHidden();
      else await expect(tree(page)).toBeVisible();

      const columns = await drawn(page);
      if (lw <= 520) expect(columns).toEqual(["title", "bpm", "key", "rating"]);
      else if (lw <= 800) expect(columns).toEqual(["title", "bpm", "key", "rating", "duration"]);
      else for (const key of ["title", "bpm", "key", "rating"]) expect(columns).toContain(key);
      await expectNoSidewaysScroll(page);

      // The list's headings and a row's cells lie inside the list, side by side.
      const list = (await scroller(page).boundingBox())!;
      await expectLaidOut(working(page), '[role="columnheader"]', list.x, list.x + list.width);
      await expectLaidOut(rows(page).first(), "[data-col]", list.x, list.x + list.width);
      // The title stays readable: never squeezed below a few characters.
      const title = (await rows(page).first().locator('[data-col="title"]').boundingBox())!;
      expect(title.width).toBeGreaterThanOrEqual(80);

      // Key lights: a lit key's badge is drawn whole inside its cell.
      const badge = scroller(page).locator('[data-col="key"] [role="img"]').first();
      await expect(badge).toBeVisible();
      const b = (await badge.boundingBox())!;
      const cell = (await scroller(page).locator('[data-col="key"]').filter({ has: page.locator('[role="img"]') }).first().boundingBox())!;
      expect(b.x).toBeGreaterThanOrEqual(cell.x - 0.5);
      expect(b.x + b.width).toBeLessThanOrEqual(cell.x + cell.width + 0.5);

      // The Track Filter bar: every control inside the window, none on another, each takes the focus.
      await expect(bar(page)).toBeVisible();
      await expectLaidOut(bar(page), "button, input, select", 0, width);
      const controls = bar(page).locator("button:not([disabled]), input:not([disabled]), select:not([disabled])");
      const count = await controls.count();
      expect(count).toBeGreaterThan(0);
      for (let i = 0; i < count; i++) {
        const control = controls.nth(i);
        await control.focus();
        await expect(control).toBeFocused();
        await expect(control).toBeInViewport();
      }

      // The shell's areas and the page itself fit the window.
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
      await expectLaidOut(page.getByTestId("shell-top-bar"), '[role="tab"], button', 0, width);
      await expectLaidOut(page.getByTestId("shell-status"), "button, span", 0, width);
    });
  }
}

for (const { width, scale } of [{ width: 1280, scale: SCALES[1]! }, { width: 1280, scale: SCALES[2]! }, { width: 900, scale: SCALES[1]! }, { width: 900, scale: SCALES[0]! }]) {
  test(`${width}px at ${Math.round(scale.factor * 100)}% with the second list: the main list drops date added, narrows MyTags and fits`, async ({ page }) => {
    await open(page, width, scale.stop, "?writable=1&filterFixture=1");
    await allTracks(page, width);
    const before = await drawn(page);
    await rows(page).first().click();
    await page.keyboard.press("Control+Backslash");
    await expect(target(page)).toBeVisible();
    const columns = await drawn(page);
    expect(columns).not.toContain("dateAdded");
    for (const key of ["title", "bpm", "key", "rating"]) expect(columns).toContain(key);
    if (before.includes("myTag") && columns.includes("myTag")) {
      const narrowed = (await rows(page).first().locator('[data-col="myTag"]').boundingBox())!.width;
      expect(narrowed).toBeLessThan(200 * scale.factor);
    }
    await expectNoSidewaysScroll(page);
    // The two lists side by side, inside the window, neither on the other.
    const main = (await scroller(page).boundingBox())!;
    const second = (await target(page).boundingBox())!;
    expect(main.x + main.width).toBeLessThanOrEqual(second.x + 0.5);
    expect(second.x + second.width).toBeLessThanOrEqual(width + 0.5);
    await expectLaidOut(target(page), "select, button, [role='row'] [data-cell]", second.x, second.x + second.width);
    // Hidden again, the main list has its columns back.
    await page.keyboard.press("Control+Backslash");
    await expect(target(page)).toBeHidden();
    await expect.poll(() => drawn(page)).toEqual(before);
  });
}

test("at 125% a 1000px window is 800 wide: the second list gives way with the tree", async ({ page }) => {
  await open(page, 1000, 6, "?writable=1&filterFixture=1");
  await allTracks(page, 1000);
  await rows(page).first().click();
  await page.keyboard.press("Control+Backslash");
  await expect(tree(page)).toBeHidden();
  await expect(target(page)).toBeHidden();
  await expect.poll(() => drawn(page)).toEqual(["title", "bpm", "key", "rating", "duration"]);
});

test("the stored column choice is untouched by a narrow window", async ({ page }) => {
  await open(page, 1280, 4, "?writable=1&filterFixture=1");
  await allTracks(page, 1280);
  const wide = await drawn(page);
  const stored = await page.evaluate(() => localStorage.getItem("rbl.columns.v2.libraryView"));
  await page.setViewportSize({ width: 436, height: 800 });
  await expect.poll(() => drawn(page)).toEqual(["title", "bpm", "key", "rating"]);
  expect(await page.evaluate(() => localStorage.getItem("rbl.columns.v2.libraryView"))).toBe(stored);
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect.poll(() => drawn(page)).toEqual(wide);
});

test("the old layout keeps its columns at 640", async ({ page }) => {
  await open(page, 1280, 4, "?writable=1&filterFixture=1", false);
  const wide = await page.locator('[data-testid="track-scroll"] [role="row"][aria-selected]').first().locator("[data-col]").evaluateAll((cells) => cells.map((c) => c.getAttribute("data-col")));
  await page.setViewportSize({ width: 640, height: 800 });
  await page.waitForTimeout(300);
  const narrow = await page.locator('[data-testid="track-scroll"] [role="row"][aria-selected]').first().locator("[data-col]").evaluateAll((cells) => cells.map((c) => c.getAttribute("data-col")));
  expect(narrow).toEqual(wide);
});
