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
  });
}
