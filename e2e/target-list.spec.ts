/**
 * The Library view's second list (wbx-59): a target playlist beside the
 * list, for building a playlist while reading another.
 *
 * Every test runs on the mock's constructed Library (`?filterFixture=1`,
 * `src/ipc/mock-filter-fixture.ts`): track i of 0 to 29 is "Bar ii", BPM
 * 120.00 + 0.25 i; "Melodic Vox" holds Bar 03 to Bar 20 in that order,
 * "Eurodance" nothing, "Fresh 128s" is an intelligent playlist. What a test
 * expects after an edit is read back through the service (the playlist
 * opened again in the main list), not only from the second list.
 */
import { expect, test, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const working = (page: Page) => page.getByRole("tabpanel");
const main = (page: Page) => working(page).getByTestId("track-scroll");
const mainRows = (page: Page) => main(page).locator('[role="row"][aria-selected]');
const mainRow = (page: Page, title: string) =>
  mainRows(page).filter({ has: page.locator('[data-col="title"]', { hasText: new RegExp(`^${title}$`) }) });
const target = (page: Page) => page.getByRole("region", { name: "Second list" });
const targetGrid = (page: Page) => target(page).getByRole("grid", { name: "Target" });
const targetRows = (page: Page) => targetGrid(page).getByRole("row").filter({ has: page.locator('[data-cell="title"]') });
const targetTitles = (page: Page) => targetGrid(page).locator('[data-cell="title"]').allInnerTexts();
const picker = (page: Page) => target(page).getByRole("combobox", { name: "Target playlist" });
const status = (page: Page) => page.getByRole("contentinfo");
const tree = (page: Page) => page.getByTestId("shell-tree");
const shellPlaylist = (page: Page, name: string) =>
  tree(page).getByRole("group", { name: "Playlists" }).getByRole("treeitem", { name, exact: true });
const menu = (page: Page, id: string) =>
  page.evaluate((item) => (window as unknown as { __menu?: (id: string) => void }).__menu?.(item), id);

const bars = (...ids: number[]) => ids.map((i) => `Bar ${String(i).padStart(2, "0")}`);
const range = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, k) => from + k);

async function open(page: Page, extra = "") {
  await enableViewShell(page);
  await page.goto(`/?writable=1&filterFixture=1${extra}`);
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
}

async function openInMain(page: Page, name: string) {
  await shellPlaylist(page, name).click();
  await expect(page.getByTestId("browser-title")).toContainText(name);
}

async function allTracks(page: Page) {
  await tree(page).getByRole("treeitem", { name: /^All Tracks/ }).click();
  await expect(page.getByTestId("browser-title")).toContainText("All Tracks");
  await expect(mainRows(page).first()).toBeVisible();
}

/** Shows the second list with `name` as its target. */
async function showTarget(page: Page, name: string) {
  await mainRows(page).first().click();
  await page.keyboard.press("Control+Backslash");
  await expect(target(page)).toBeVisible();
  // Offered under its folders' names ("CURRENT / Melodic Vox").
  const label = (await picker(page).locator("option").filter({ hasText: new RegExp(`(^| / )${name}$`) }).first().innerText()).trim();
  await picker(page).selectOption({ label });
  await expect(target(page).getByRole("heading")).toContainText(name);
}

/** A playlist's tracks as the main list lists them once it is opened. */
async function playlistAsListed(page: Page, name: string): Promise<string[]> {
  await openInMain(page, name);
  return main(page).locator('[data-col="title"]').allInnerTexts().then((all) => all.map((title) => title.trim()));
}

test("the second list is hidden by default, and one key shows and hides it", async ({ page }) => {
  await open(page);
  await allTracks(page);
  await expect(target(page)).toHaveCount(0);

  await mainRows(page).first().click();
  await page.keyboard.press("Control+Backslash");
  await expect(target(page)).toBeVisible();
  await page.keyboard.press("Control+Backslash");
  await expect(target(page)).toHaveCount(0);
});

test("it shows title, BPM, key and Energy, with the target's track count and length", async ({ page }) => {
  await open(page);
  // The length the main list gives Melodic Vox's tracks, read from its Time cells.
  await openInMain(page, "Melodic Vox");
  const times = (await main(page).locator('[data-col="duration"]').allInnerTexts()).map((text) => {
    const [m, s] = text.trim().split(":").map(Number);
    return (m ?? 0) * 60 + (s ?? 0);
  });
  expect(times).toHaveLength(18);
  const minutes = Math.floor(times.reduce((sum, value) => sum + value, 0) / 60);
  const length = minutes >= 60 ? `${Math.floor(minutes / 60)}h${String(minutes % 60).padStart(2, "0")}m` : `${minutes}m`;

  await allTracks(page);
  await showTarget(page, "Melodic Vox");
  await expect(targetGrid(page).getByRole("columnheader")).toHaveText(["Title", "BPM", "Key", "Energy"]);
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
  await expect(target(page).getByRole("heading")).toContainText(`18 tracks · ${length}`);
  // Bar 03 is 120.75 BPM, key 8A, Energy 1 (the fixture's formulas).
  const first = targetRows(page).first();
  await expect(first.locator('[data-cell="bpm"]')).toHaveText("120.75");
  await expect(first.locator('[data-cell="key"]')).toContainText("8A");
  await expect(first.getByRole("img", { name: "Energy 1 of 5" })).toBeVisible();
});
