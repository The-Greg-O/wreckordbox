import { expect, test, type Locator, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

/**
 * Three defects the DJ hit hands-on (2026-11-15), each in the old layout and
 * the view shell: wbx-w41 (the list stuck after Undo takes away the open
 * playlist), wbx-iwl (clicks shifted once the window is narrower than its
 * minimum width) and wbx-52f (the full-track waveform needs two clicks to seek).
 */

const menu = (page: Page, id: string) =>
  page.evaluate((m) => (window as unknown as { __menu?: (id: string) => void }).__menu?.(m), id);

function item(page: Page, name: string): Locator {
  return page.getByRole("treeitem").filter({ hasText: name }).first();
}

const title = (page: Page) => page.getByTestId("browser-title");
const rows = (page: Page) => page.getByRole("row").filter({ has: page.getByRole("gridcell") });
const titles = (page: Page) => rows(page).locator('[data-col="title"]').allInnerTexts();

/** The full-track waveform of the one deck each layout draws. */
const overviewOf = (page: Page, layout: string) => layout === "view shell"
  ? page.getByTestId("shell-player").getByTestId("compact-player-overview")
  : page.getByTestId("player-overview").first();

/** Clicks once at `fraction` across the overview and gives where the deck says it is, over the length. */
async function clickAt(page: Page, overview: Locator, fraction: number): Promise<void> {
  const box = await overview.boundingBox();
  if (!box) throw new Error("the overview is not drawn");
  await page.mouse.click(box.x + box.width * fraction, box.y + box.height / 2);
}
const positionOf = async (overview: Locator) => {
  const [now, max] = await Promise.all([overview.getAttribute("aria-valuenow"), overview.getAttribute("aria-valuemax")]);
  return Number(now) / Number(max);
};

for (const layout of ["old layout", "view shell"] as const) {
  test.describe(layout, () => {
    test.beforeEach(async ({ page }) => {
      if (layout === "view shell") await enableViewShell(page);
    });

    test("wbx-w41: Undo that takes away the open playlist leaves a list that follows the tree", async ({ page }) => {
      await page.goto("/?writable=1");
      await expect(title(page)).toContainText("Tracks)");
      await item(page, "Melodic Vox").click({ button: "right" });
      await page.getByRole("menu", { name: "Playlist" }).getByRole("menuitem", { name: "Create New Playlist", exact: true }).click();
      const made = item(page, "New playlist");
      await expect(made).toBeVisible();
      await made.click();
      await expect(rows(page)).toHaveCount(0);

      await menu(page, "undo");
      await expect(page.getByRole("treeitem").filter({ hasText: "New playlist" })).toHaveCount(0);
      // The list does not go on naming a playlist the Library no longer has.
      await expect(title(page)).not.toContainText("New playlist");
      await expect(title(page)).toContainText("All Tracks");
      await expect(page.getByText("New playlist is no longer in the Library.")).toBeVisible();

      // And choosing another playlist opens it.
      await item(page, "Melodic Vox").click();
      await expect(title(page)).toContainText("Melodic Vox");
      await expect.poll(async () => (await titles(page)).length).toBeGreaterThan(0);
    });

    test("wbx-w41: a list that cannot be read says so instead of showing the list before, and the next choice reads", async ({ page }) => {
      await page.goto("/?writable=1");
      await item(page, "Melodic Vox").click();
      await expect(title(page)).toContainText("Melodic Vox");
      const before = await titles(page);
      expect(before.length).toBeGreaterThan(0);
      await page.evaluate(() => { (window as unknown as { __failOpenView?: boolean }).__failOpenView = true; });
      await item(page, "Eurodance").click();
      await expect(page.getByTestId("list-error")).toContainText("could not be read");
      await expect(rows(page)).toHaveCount(0);
      await page.evaluate(() => { (window as unknown as { __failOpenView?: boolean }).__failOpenView = false; });
      await item(page, "Melodic Vox").click();
      await expect(page.getByTestId("list-error")).toHaveCount(0);
      await expect.poll(() => titles(page)).toEqual(before);
    });

    test("wbx-52f: one click on the full-track waveform moves the deck there", async ({ page }) => {
      await page.goto("/");
      await rows(page).nth(3).locator('[data-col="title"]').dblclick();
      const overview = overviewOf(page, layout);
      await expect(overview).toHaveAttribute("aria-valuemax", /^[1-9]/);
      await clickAt(page, overview, 0.5);
      await expect.poll(() => positionOf(overview)).toBeCloseTo(0.5, 1);
      await clickAt(page, overview, 0.2);
      await expect.poll(() => positionOf(overview)).toBeCloseTo(0.2, 1);
    });
  });
}

test.describe("view shell, narrow", () => {
  test.beforeEach(async ({ page }) => enableViewShell(page));
  for (const width of [640, 436, 360]) {
    test(`wbx-iwl: at ${width} px a click on the full-track waveform lands under the pointer`, async ({ page }) => {
      await page.setViewportSize({ width, height: 700 });
      await page.goto("/");
      await rows(page).nth(3).locator('[data-col="title"]').dblclick();
      const overview = overviewOf(page, "view shell");
      await expect(overview).toHaveAttribute("aria-valuemax", /^[1-9]/);
      for (const fraction of [0.25, 0.75]) {
        await clickAt(page, overview, fraction);
        await expect.poll(() => positionOf(overview)).toBeCloseTo(fraction, 1);
      }
      // Nothing is laid out wider than the window, so nothing can be squeezed.
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
    });
  }
});
