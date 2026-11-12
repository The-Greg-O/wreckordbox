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

test("Enter adds the selected track to the target, as one Undo step, and the main list still loads without the second list", async ({ page }) => {
  await open(page);
  await allTracks(page);
  await showTarget(page, "Eurodance");
  await expect(target(page).getByText(/Empty\./)).toBeVisible();

  await mainRow(page, "Bar 05").locator('[data-col="title"]').click();
  await page.keyboard.press("Enter");
  await expect(status(page)).toContainText("Added 1 track to Eurodance.");
  await expect.poll(() => targetTitles(page)).toEqual(bars(5));
  await expect(target(page).getByRole("heading")).toContainText("1 tracks");
  // Enter added: it did not also load the track on the deck.
  await expect(page.getByTestId("shell-player")).not.toContainText("Bar 05");

  await mainRow(page, "Bar 09").locator('[data-col="title"]').click();
  await page.keyboard.press("Enter");
  await expect.poll(() => targetTitles(page)).toEqual(bars(5, 9));

  // The Library holds it: the playlist opened in the main list lists both.
  expect(await playlistAsListed(page, "Eurodance")).toEqual(bars(5, 9));

  // One Undo takes back the last add, and only it.
  await menu(page, "undo");
  await expect(status(page)).toContainText("Edit undone.");
  await expect.poll(() => targetTitles(page)).toEqual(bars(5));

  // With the second list hidden, Enter is not an add: it is left to the
  // window's own handlers (Load on Player 1), and the target is unchanged.
  await allTracks(page);
  await mainRow(page, "Bar 11").locator('[data-col="title"]').click();
  await page.keyboard.press("Control+Backslash");
  await expect(target(page)).toHaveCount(0);
  const prevented = await page.evaluate(() => {
    const press = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
    document.body.dispatchEvent(press);
    return press.defaultPrevented;
  });
  expect(prevented).toBe(false);
  await page.keyboard.press("Enter");
  await page.keyboard.press("Control+Backslash");
  await expect.poll(() => targetTitles(page)).toEqual(bars(5));
  expect(await playlistAsListed(page, "Eurodance")).toEqual(bars(5));
});

test("dragging tracks from the main list onto the second list adds them", async ({ page }) => {
  await open(page);
  await allTracks(page);
  await showTarget(page, "Eurodance");

  await mainRow(page, "Bar 02").locator('[data-col="title"]').click();
  await mainRow(page, "Bar 04").locator('[data-col="title"]').click({ modifiers: ["Shift"] });
  await mainRow(page, "Bar 03").dragTo(targetGrid(page));
  await expect(status(page)).toContainText("Added 3 tracks to Eurodance.");
  await expect.poll(() => targetTitles(page)).toEqual(bars(2, 3, 4));
  expect(await playlistAsListed(page, "Eurodance")).toEqual(bars(2, 3, 4));
});

test("under Library Protection an add is refused, saying why, and the target is unchanged", async ({ page }) => {
  await open(page);
  await allTracks(page);
  await showTarget(page, "Melodic Vox");
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
  await page.evaluate(() => (window as unknown as { __turnLibraryProtection: (on: boolean) => void }).__turnLibraryProtection(true));

  await mainRow(page, "Bar 25").locator('[data-col="title"]').click();
  await page.keyboard.press("Enter");
  await expect(status(page)).toContainText("Editing is locked by Library Protection");
  await mainRow(page, "Bar 26").dragTo(targetGrid(page));
  await expect(status(page)).toContainText("Editing is locked by Library Protection");
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
  expect(await playlistAsListed(page, "Melodic Vox")).toEqual(bars(...range(3, 20)));
});

/** Melodic Vox as the target, with Bar 03, Bar 04 and Bar 05 its first three. */
async function voxTarget(page: Page) {
  await open(page);
  await allTracks(page);
  await showTarget(page, "Melodic Vox");
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
}

test("Alt+Down and Alt+Up reorder the target's selected track, the selection follows it, and Undo puts it back", async ({ page }) => {
  await voxTarget(page);
  await targetRows(page).nth(0).click();
  await page.keyboard.press("Alt+ArrowDown");
  await expect(status(page)).toContainText("Playlist reordered.");
  await expect.poll(() => targetTitles(page)).toEqual(bars(4, 3, ...range(5, 20)));
  await expect(targetRows(page).nth(1)).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Alt+ArrowDown");
  await expect.poll(() => targetTitles(page)).toEqual(bars(4, 5, 3, ...range(6, 20)));
  await page.keyboard.press("Alt+ArrowUp");
  await expect.poll(() => targetTitles(page)).toEqual(bars(4, 3, ...range(5, 20)));

  // The reorder is the Library's: the playlist opened in the main list has it.
  expect(await playlistAsListed(page, "Melodic Vox")).toEqual(bars(4, 3, ...range(5, 20)));

  await menu(page, "undo");
  await expect(status(page)).toContainText("Edit undone.");
  await expect.poll(() => targetTitles(page)).toEqual(bars(4, 5, 3, ...range(6, 20)));
});

test("plain arrows move the target's selection, not the main list's, and Enter and Space in the target reach neither the list nor the deck", async ({ page }) => {
  await voxTarget(page);
  await mainRow(page, "Bar 00").locator('[data-col="title"]').click();
  await targetRows(page).nth(0).click();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(targetRows(page).nth(2)).toHaveAttribute("aria-selected", "true");
  await expect(mainRow(page, "Bar 00")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Space");
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
  await expect(page.getByTestId("shell-player")).toContainText("No track loaded");
});

test("dragging a row within the target reorders it", async ({ page }) => {
  await voxTarget(page);
  // Bar 05 dropped on the top half of Bar 03: it goes first.
  const onto = targetRows(page).nth(0);
  const box = await onto.boundingBox();
  if (!box) throw new Error("The target's first row is not visible");
  await targetRows(page).nth(2).dragTo(onto, { targetPosition: { x: 20, y: 2 } });
  await expect(status(page)).toContainText("Playlist reordered.");
  await expect.poll(() => targetTitles(page)).toEqual(bars(5, 3, 4, ...range(6, 20)));
  expect(await playlistAsListed(page, "Melodic Vox")).toEqual(bars(5, 3, 4, ...range(6, 20)));
});

test("one key swaps source and target, and the tree marks the target", async ({ page }) => {
  await open(page);
  await openInMain(page, "Melodic Vox");
  await mainRows(page).first().click();
  await page.keyboard.press("Control+Backslash");
  await picker(page).selectOption({ label: (await picker(page).locator("option").filter({ hasText: /Eurodance$/ }).innerText()).trim() });
  await expect(shellPlaylist(page, "Eurodance")).toHaveAccessibleDescription("Target playlist");
  await expect(shellPlaylist(page, "Melodic Vox")).not.toHaveAccessibleDescription("Target playlist");

  await page.keyboard.press("Control+Shift+Backslash");
  await expect(page.getByTestId("browser-title")).toContainText("Eurodance");
  await expect(target(page).getByRole("heading")).toContainText("Melodic Vox");
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
  await expect(shellPlaylist(page, "Melodic Vox")).toHaveAccessibleDescription("Target playlist");
  await expect(shellPlaylist(page, "Eurodance")).not.toHaveAccessibleDescription("Target playlist");

  // Hidden, nothing in the tree is marked.
  await page.keyboard.press("Control+Backslash");
  await expect(tree(page).locator("[aria-description]")).toHaveCount(0);
});

test("a Stick's playlist is never a target: not offered, a swap from it is refused, and its tracks are not added", async ({ page }) => {
  await open(page);
  await allTracks(page);
  await showTarget(page, "Eurodance");
  await expect(picker(page).locator("option", { hasText: "Now Playing Test" })).toHaveCount(0);

  const stickPlaylist = tree(page).getByRole("group", { name: "Stick TEST" }).getByRole("treeitem", { name: "Now Playing Test" });
  await stickPlaylist.click();
  await expect(page.getByTestId("browser-title")).toContainText("Now Playing Test");
  const before = await main(page).locator('[data-col="title"]').allInnerTexts();

  await page.keyboard.press("Control+Shift+Backslash");
  await expect(target(page).getByRole("status")).toContainText("A Stick's playlist cannot be the target");
  await expect(page.getByTestId("browser-title")).toContainText("Now Playing Test");
  await expect(target(page).getByRole("heading")).toContainText("Eurodance");

  await mainRows(page).first().locator('[data-col="title"]').click();
  await page.keyboard.press("Enter");
  await expect(target(page).getByRole("status")).toContainText("Tracks listed from a Stick cannot be added");
  await mainRows(page).nth(1).dragTo(targetGrid(page));
  await expect(target(page).getByRole("status")).toContainText("Tracks listed from a Stick cannot be added");
  await expect.poll(() => targetTitles(page)).toEqual([]);
  expect(await main(page).locator('[data-col="title"]').allInnerTexts()).toEqual(before);
});

test("at half width the second list gives way, and comes back when the window is wide again", async ({ page }) => {
  await voxTarget(page);
  await page.setViewportSize({ width: 640, height: 800 });
  await expect(target(page)).toHaveCount(0);
  // Enter is not an add while the list is not there.
  await mainRow(page, "Bar 00").locator('[data-col="title"]').click();
  await page.keyboard.press("Enter");
  await page.keyboard.press("Control+Backslash");
  await expect(status(page)).toContainText("too narrow for a second list");
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(target(page)).toBeVisible();
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
});
