/**
 * The Library at half width without the tree (wbx-45.21;
 * docs/design/screens/library "Sizes": "no tree (the playlist name becomes
 * a breadcrumb)"). At 800 CSS px and below the list's heading is a
 * breadcrumb (the open list's folders and name) that opens the tree over
 * the view; "Go to the Tree" (palette, Ctrl+T) does the same from the
 * keyboard. Choosing a row there opens it and closes the tree; Escape
 * closes it. The tree's selection, the second list's target and the Track
 * Filter survive. Bounds, not text, at 640 and 436 px and 0.8, 1, 1.25.
 *
 * Mock: `?filterFixture=1` (folder CURRENT holds Melodic Vox, Hardstyle...).
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { DEFAULT_PREFERENCES } from "../src/lib/preferences";

const SCALES = [{ stop: 2, factor: 0.8 }, { stop: 4, factor: 1 }, { stop: 6, factor: 1.25 }];

const working = (page: Page) => page.getByRole("tabpanel");
const crumb = (page: Page) => working(page).getByRole("button", { name: /^Open the tree/ });
const treePane = (page: Page) => page.getByTestId("shell-tree");
const tree = (page: Page) => page.getByRole("tree", { name: "Tree" });
const playlist = (page: Page, name: string) => tree(page).getByRole("group", { name: "Playlists" }).getByRole("treeitem", { name, exact: true });
const selectedRow = (page: Page) => tree(page).locator('[role="treeitem"][aria-selected="true"]');
const rows = (page: Page) => working(page).getByTestId("track-scroll").locator('[role="row"][aria-selected]');
const bar = (page: Page) => page.getByRole("search", { name: "Track Filter" });
const target = (page: Page) => page.getByRole("region", { name: "Second list" });

async function open(page: Page, width: number, stop: number): Promise<void> {
  await page.setViewportSize({ width: 1600, height: 800 });
  await page.addInitScript((full) => {
    if (sessionStorage.getItem("crumb-seeded")) return;
    sessionStorage.setItem("crumb-seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify(full));
  }, { ...DEFAULT_PREFERENCES, advanced: { ...DEFAULT_PREFERENCES.advanced, protectLibrary: false },
    view: { ...DEFAULT_PREFERENCES.view, viewShell: true, uiScale: stop } });
  await page.goto("/?writable=1&filterFixture=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await playlist(page, "Melodic Vox").click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await expect(rows(page).first()).toBeVisible();
  await page.setViewportSize({ width, height: 800 });
  if (width / ([0, 0, 0.8, 0, 1, 0, 1.25][stop] ?? 1) <= 800) await expect(treePane(page)).toBeHidden();
}

type Box = { x: number; y: number; width: number; height: number };
const inside = (a: Box, b: Box) => a.x >= b.x - 0.5 && a.y >= b.y - 0.5 && a.x + a.width <= b.x + b.width + 0.5 && a.y + a.height <= b.y + b.height + 0.5;
const box = async (l: Locator) => (await l.boundingBox())!;

/** The breadcrumb's parts lie inside it, side by side, and it lies inside the list. */
async function expectCrumbLaidOut(page: Page, width: number): Promise<void> {
  await expect(crumb(page)).toBeVisible();
  const c = await box(crumb(page));
  expect(c.x).toBeGreaterThanOrEqual(-0.5);
  expect(c.x + c.width).toBeLessThanOrEqual(width + 0.5);
  const parts = await crumb(page).locator("[data-crumb]").evaluateAll((els) => els.map((e) => { const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height }; }));
  // CURRENT, then Melodic Vox.
  expect(parts.length).toBe(2);
  for (const p of parts) expect(inside(p, c)).toBe(true);
  expect(parts[0]!.x + parts[0]!.width).toBeLessThanOrEqual(parts[1]!.x + 0.5);
  // It is the list's heading: the old title row is not drawn beside it.
  await expect(page.getByTestId("browser-title")).toBeHidden();
  // The rest of the list's head stays: its search, within the window, below the breadcrumb.
  await expect(working(page).getByRole("searchbox")).toBeVisible();
  const search = await box(working(page).getByRole("searchbox"));
  expect(search.x).toBeGreaterThanOrEqual(-0.5);
  expect(search.x + search.width).toBeLessThanOrEqual(width + 0.5);
  expect(search.y).toBeGreaterThanOrEqual(c.y + c.height - 0.5);
  // Above the Track Filter bar, which stays.
  const b = await box(bar(page));
  expect(c.y + c.height).toBeLessThanOrEqual(b.y + 0.5);
}

/** The tree over the view: inside the window, between the top bar and the status line. */
async function expectTreeOver(page: Page, width: number): Promise<void> {
  await expect(treePane(page)).toBeVisible();
  const t = await box(treePane(page));
  const top = await box(page.getByTestId("shell-top-bar"));
  const status = await box(page.getByTestId("shell-status"));
  expect(t.x).toBeGreaterThanOrEqual(-0.5);
  expect(t.x + t.width).toBeLessThanOrEqual(width + 0.5);
  expect(t.y).toBeGreaterThanOrEqual(top.y + top.height - 0.5);
  expect(t.y + t.height).toBeLessThanOrEqual(status.y + 0.5);
  expect(t.width).toBeGreaterThanOrEqual(Math.min(186, width - 1));
  // Its rows are drawn and readable within it.
  const first = await box(playlist(page, "Hardstyle"));
  expect(inside(first, t)).toBe(true);
}

for (const width of [640, 436]) {
  for (const scale of SCALES) {
    test(`${width}px at ${Math.round(scale.factor * 100)}%: the breadcrumb, and the tree by keyboard`, async ({ page }) => {
      await open(page, width, scale.stop);
      await expectCrumbLaidOut(page, width);

      // Ctrl+T from the list: the tree opens over the view with the open list's row focused.
      await rows(page).first().click();
      await page.keyboard.press("Control+t");
      await expectTreeOver(page, width);
      await expect(playlist(page, "Melodic Vox")).toBeFocused();
      // Down to the next playlist and Enter: it opens and the tree closes.
      await page.keyboard.press("ArrowDown");
      await expect(playlist(page, "Hardstyle")).toBeFocused();
      await page.keyboard.press("Enter");
      await expect(treePane(page)).toBeHidden();
      await expect(page.getByTestId("browser-title")).toContainText("Hardstyle");
      await expect(crumb(page)).toBeFocused();

      // Enter on the breadcrumb opens it again; Escape closes it with nothing changed.
      await page.keyboard.press("Enter");
      await expectTreeOver(page, width);
      await expect(playlist(page, "Hardstyle")).toBeFocused();
      await page.keyboard.press("Escape");
      await expect(treePane(page)).toBeHidden();
      await expect(page.getByTestId("browser-title")).toContainText("Hardstyle");
      await expect(crumb(page)).toBeFocused();
      await expectCrumbLaidOut(page, width);
    });
  }
}

test("by pointer: the breadcrumb opens the tree, a row opens its list, a click outside closes it", async ({ page }) => {
  await open(page, 640, 4);
  await crumb(page).click();
  await expectTreeOver(page, 640);
  await playlist(page, "Hardstyle").click();
  await expect(treePane(page)).toBeHidden();
  await expect(page.getByTestId("browser-title")).toContainText("Hardstyle");
  await crumb(page).click();
  await expectTreeOver(page, 640);
  // A folder's twisty opens or closes the folder; the tree stays.
  await tree(page).getByRole("button", { name: /^Collapse CURRENT/ }).click();
  await expect(treePane(page)).toBeVisible();
  await tree(page).getByRole("button", { name: /^Expand CURRENT/ }).click();
  await page.mouse.click(620, 700);
  await expect(treePane(page)).toBeHidden();
  await expect(page.getByTestId("browser-title")).toContainText("Hardstyle");
});

test("the tree's selection, the target and the Track Filter survive the narrow tree", async ({ page }) => {
  await open(page, 1280, 4);
  // A target and a filter at full width.
  await rows(page).first().click();
  await page.keyboard.press("Control+Backslash");
  await expect(target(page)).toBeVisible();
  const picker = target(page).getByRole("combobox", { name: "Target playlist" });
  await picker.selectOption({ label: (await picker.locator("option").filter({ hasText: /Eurodance$/ }).first().innerText()).trim() });
  await expect(playlist(page, "Eurodance")).toHaveAccessibleDescription("Target playlist");
  await bar(page).getByRole("textbox", { name: "Filter text" }).fill("Bar");
  const count = await bar(page).getByRole("status").innerText();

  await page.setViewportSize({ width: 640, height: 800 });
  await expect(target(page)).toBeHidden();
  // From the list (in the filter's text field Ctrl+T is the field's).
  await rows(page).first().click();
  await page.keyboard.press("Control+t");
  await expectTreeOver(page, 640);
  await playlist(page, "Hardstyle").click();
  await expect(treePane(page)).toBeHidden();
  await expect(bar(page).getByRole("textbox", { name: "Filter text" })).toHaveValue("Bar");

  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(treePane(page)).toBeVisible();
  await expect(selectedRow(page)).toHaveCount(1);
  await expect(selectedRow(page)).toHaveAttribute("aria-label", "Hardstyle");
  await expect(target(page)).toBeVisible();
  await expect(playlist(page, "Eurodance")).toHaveAccessibleDescription("Target playlist");
  await expect(bar(page).getByRole("textbox", { name: "Filter text" })).toHaveValue("Bar");
  expect(count.length).toBeGreaterThan(0);
  // At full width the breadcrumb gives way to the list's own heading.
  await expect(crumb(page)).toBeHidden();
  await expect(page.getByTestId("browser-title")).toBeVisible();
});

test("the palette has Go to the Tree: at half width it opens the tree, at full width it focuses it", async ({ page }) => {
  await open(page, 640, 4);
  const palette = page.getByRole("dialog", { name: "Command palette" });
  const run = async () => {
    await page.keyboard.press("Control+Shift+P");
    await palette.getByRole("combobox", { name: "Search actions" }).fill("Go to the Tree");
    await palette.getByRole("listbox", { name: "Actions" }).getByRole("option", { name: /^Go to the Tree/ }).first().click();
    await expect(palette).toHaveCount(0);
  };
  await run();
  await expectTreeOver(page, 640);
  await expect(playlist(page, "Melodic Vox")).toBeFocused();
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 1280, height: 800 });
  await run();
  await expect(playlist(page, "Melodic Vox")).toBeFocused();
});

test("Ctrl+T in a text field is the field's", async ({ page }) => {
  await open(page, 640, 4);
  await bar(page).getByRole("textbox", { name: "Filter text" }).focus();
  await page.keyboard.press("Control+t");
  await page.waitForTimeout(200);
  await expect(treePane(page)).toBeHidden();
});
