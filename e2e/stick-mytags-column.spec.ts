/**
 * The MyTags column on a Stick list (wbx-f7y).
 *
 * A Stick row's MyTags cell shows the names of the Stick's own MyTags, the
 * way a Library row shows the Library's, read without writing the Stick;
 * the cell offers no edit.
 *
 * The mock Stick (`src/ipc/mock-stick-mytags.ts`): "Vibe" (Dark, Uplifting)
 * and "Situation" (Peak, Warm-up). "Now Playing Test" holds B2ME (Uplifting,
 * Peak), Spectrum (Dark, Uplifting, Peak), Lost Without You (Dark, Warm-up)
 * and Gravity (none). The Library is the filter fixture, whose tags differ.
 */
import { expect, test, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const working = (page: Page) => page.getByRole("tabpanel");
const rows = (page: Page) => working(page).locator('[role="row"][aria-selected]');
const tree = (page: Page) => page.getByTestId("shell-tree");
const bar = (page: Page) => page.getByRole("search", { name: "Track Filter" });
const tagsOf = (page: Page, title: string) =>
  rows(page).filter({ has: page.locator('[data-col="title"]', { hasText: title }) }).locator('[data-col="myTag"]');

type Hooks = { __stickWrites: () => { method: string; path: string }[] };
const stickWrites = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__stickWrites());

async function openStickList(page: Page) {
  await page.setViewportSize({ width: 1280, height: 800 });
  await enableViewShell(page);
  await page.goto("/?writable=1&filterFixture=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await tree(page).getByRole("group", { name: "Stick TEST" }).getByRole("treeitem", { name: "Now Playing Test" }).click();
  await expect(page.getByTestId("browser-title")).toContainText("Now Playing Test");
  await expect(rows(page)).toHaveCount(4);
}

test("a Stick row's MyTags cell names the Stick's own MyTags, with their categories", async ({ page }) => {
  await openStickList(page);
  const spectrum = tagsOf(page, "Spectrum (Extended Mix)").locator("[data-tag]");
  await expect(spectrum).toHaveText(["Dark", "Uplifting", "Peak"]);
  await expect(spectrum.nth(2)).toHaveAttribute("title", "Situation: Peak");
  await expect(tagsOf(page, "B2ME (feat. JordinLaine) (Original Mix)").locator("[data-tag]")).toHaveText(["Uplifting", "Peak"]);
  await expect(tagsOf(page, "Lost Without You").locator("[data-tag]")).toHaveText(["Dark", "Warm-up"]);
  await expect(tagsOf(page, "Gravity").locator("[data-tag]")).toHaveCount(0);
  // Narrowed by a tag, the rows still name theirs.
  await bar(page).getByRole("button", { name: /^Vibe/ }).first().click();
  await bar(page).getByRole("group", { name: "Vibe tags", exact: true }).getByRole("button", { name: "Dark", exact: true }).click();
  await expect(rows(page)).toHaveCount(2);
  await expect(tagsOf(page, "Lost Without You").locator("[data-tag]")).toHaveText(["Dark", "Warm-up"]);
  expect(await stickWrites(page)).toEqual([]);
  const dir = process.env.SHOTS_DIR;
  if (dir) await page.screenshot({ path: `${dir}/stick-mytags-column-1280.png` });
});

test("a Stick row's MyTags cannot be edited from its cell", async ({ page }) => {
  await openStickList(page);
  const cell = tagsOf(page, "Spectrum (Extended Mix)");
  await expect(cell.locator("[data-tag]")).toHaveText(["Dark", "Uplifting", "Peak"]);
  // Nothing in the cell takes input.
  await expect(cell.locator("button, input, textarea, select, [contenteditable], [tabindex]")).toHaveCount(0);
  // Pressing on it, twice, and the key an editor would answer open nothing.
  await cell.click();
  await cell.dblclick();
  await page.keyboard.press("F2");
  await expect(page.getByRole("textbox", { name: /tag/i })).toHaveCount(0);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(cell.locator("[data-tag]")).toHaveText(["Dark", "Uplifting", "Peak"]);
  expect(await stickWrites(page)).toEqual([]);
});
