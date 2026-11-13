/**
 * The Library list kept across a view switch (wbx-45.2).
 *
 * While another view shows, the Library's list stays mounted but hidden, so
 * its row selection and scroll position survive the round trip. Hidden, it
 * is inert: no list key, list command, Delete, Analyze or Track menu item
 * acts on the selection nobody can see. Back in Library, the rows shown
 * selected are again what Analyze and the commands act on.
 */
import { expect, test, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

type Page$ = { __confirmed?: string[] };

const working = (page: Page) => page.getByRole("tabpanel");
const rows = (page: Page) => page.getByRole("row").filter({ has: page.getByRole("gridcell") });
const selectedRows = (page: Page) => rows(page).and(page.locator("[data-selected]"));
const selectedTitles = (page: Page) => selectedRows(page).locator('[role="gridcell"][data-col="title"]').allInnerTexts();
const analysis = (page: Page) => page.getByRole("dialog", { name: "Analysis Setting" });
const palette = (page: Page) => page.getByRole("dialog", { name: "Command palette" });
const paletteOption = (page: Page, name: RegExp) =>
  palette(page).getByRole("listbox", { name: "Actions" }).getByRole("option", { name });
const asked = (page: Page) => page.evaluate(() => (window as unknown as Page$).__confirmed ?? []);
const scrollTop = (page: Page) => page.getByTestId("track-scroll").evaluate((el) => el.scrollTop);
const shellPlaylist = (page: Page, name: string) =>
  page.getByTestId("shell-tree").getByRole("group", { name: "Playlists" }).getByRole("treeitem", { name });

async function toIntake(page: Page) {
  await page.keyboard.press("2");
  await expect(working(page).getByRole("heading", { name: "Intake" })).toBeVisible();
}

async function toLibrary(page: Page) {
  await page.keyboard.press("1");
  await expect(page.getByTestId("browser-title")).toBeVisible();
}

async function analyseCount(page: Page): Promise<string> {
  await page.keyboard.press("Control+Shift+A");
  await expect(analysis(page)).toBeVisible();
  const text = await analysis(page).innerText();
  await analysis(page).getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(analysis(page)).toHaveCount(0);
  return text;
}

test("away and back, the list keeps its selected rows and where it was scrolled, and Analyze takes them", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/?writable=1&tracks=400");
  await page.getByRole("treeitem", { name: /All Tracks/ }).click();
  await expect(page.getByTestId("browser-title")).toContainText("All Tracks");

  await page.getByTestId("track-scroll").evaluate((el) => { el.scrollTop = 2400; });
  await expect.poll(() => scrollTop(page)).toBeGreaterThan(2000);
  // Rows picked by title: the list is virtual, so their order in the page is not the list's.
  const shown = await rows(page).locator('[role="gridcell"][data-col="title"]').allInnerTexts();
  const picks = [shown[8], shown[9], shown[11], shown[12]].map((title) => title ?? "");
  for (const [n, title] of picks.entries()) {
    await rows(page).filter({ hasText: title }).first().click(n === 0 ? {} : { modifiers: ["Control"] });
  }
  await expect(selectedRows(page)).toHaveCount(4);
  const chosen = await selectedTitles(page);
  expect([...chosen].sort()).toEqual([...picks].sort());
  const at = await scrollTop(page);
  expect(at).toBeGreaterThan(1000);

  await toIntake(page);
  // The list is out of sight: none of its rows can be reached.
  await expect(page.getByTestId("track-scroll").getByRole("row")).toHaveCount(0);
  await toLibrary(page);

  expect(await scrollTop(page)).toBe(at);
  await expect(selectedRows(page)).toHaveCount(4);
  expect(await selectedTitles(page)).toEqual(chosen);
  expect(await analyseCount(page)).toContain("4 tracks selected");
});

test("while another view shows, nothing acts on the hidden list's selection; back in Library it does again", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await shellPlaylist(page, "Melodic Vox").click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  const before = await rows(page).count();
  await rows(page).nth(1).click();
  await rows(page).nth(2).click({ modifiers: ["Shift"] });
  await expect(selectedRows(page)).toHaveCount(2);
  const chosen = await selectedTitles(page);

  await toIntake(page);
  // Analyze has nothing to act on.
  await page.keyboard.press("Control+Shift+A");
  await page.waitForTimeout(500);
  await expect(analysis(page)).toHaveCount(0);
  // Delete asks nothing and removes nothing.
  await page.locator("body").focus();
  await page.keyboard.press("Delete");
  await page.waitForTimeout(500);
  expect(await asked(page)).toEqual([]);
  // The list's keys move nothing: arrows, select all.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Control+a");
  // The Track menu's items are not offered for it.
  await page.keyboard.press("Control+Shift+P");
  await expect(palette(page)).toBeVisible();
  await palette(page).getByRole("combobox", { name: "Search actions" }).fill("remove from collection");
  await expect(paletteOption(page, /^Remove from Collection/)).toHaveAttribute("aria-disabled", "true");
  await expect(paletteOption(page, /^Remove from Collection/)).toContainText("Select tracks in a list first");
  await page.keyboard.press("Escape");
  await expect(palette(page)).toHaveCount(0);

  await toLibrary(page);
  await expect(rows(page)).toHaveCount(before);
  await expect(selectedRows(page)).toHaveCount(2);
  expect(await selectedTitles(page)).toEqual(chosen);
  // Shown again, its selection is the one commands act on.
  await page.keyboard.press("Control+Shift+P");
  await palette(page).getByRole("combobox", { name: "Search actions" }).fill("remove from collection");
  await expect(paletteOption(page, /^Remove from Collection/)).not.toHaveAttribute("aria-disabled", "true");
  await page.keyboard.press("Escape");
  expect(await analyseCount(page)).toContain("2 tracks selected");
  // And its keys are its own again.
  await page.keyboard.press("ArrowDown");
  await expect(selectedRows(page)).toHaveCount(1);
});
