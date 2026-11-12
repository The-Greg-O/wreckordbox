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

  // With the second list hidden, Enter is the deck's again: Load on Player 1
  // loads the row selected in the main list, which showing and hiding the
  // second list left selected.
  await allTracks(page);
  await mainRow(page, "Bar 11").locator('[data-col="title"]').click();
  await page.keyboard.press("Control+Backslash");
  await expect(target(page)).toHaveCount(0);
  await expect(mainRow(page, "Bar 11")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("compact-player-title")).toHaveText("Bar 11");
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

type Hooks = {
  __playlistEntries: (name: string) => { entryId: string; title: string }[];
  __holdEntryReorders: () => void;
  __releaseEntryReorders: () => void;
  __failNextEntryReorder: boolean;
  __entryReorders: string[][];
  __failPlaylistEntries: boolean;
  __renamePlaylistOutside: (name: string, to: string) => void;
  __deletePlaylistOutside: (name: string) => void;
  __turnLibraryProtection: (on: boolean) => void;
};
/** The playlist's entries as the Library holds them: identity and track, in order. */
const entries = (page: Page, name: string) =>
  page.evaluate((playlist) => (window as unknown as Hooks).__playlistEntries(playlist), name);
const entryTitles = async (page: Page, name: string) => (await entries(page, name)).map((entry) => entry.title);
/** Runs `run` on the page's window, where the mock's test hooks are. */
const hooks = <T,>(page: Page, run: (h: Hooks) => T) => page.evaluate<T>(`(${run.toString()})(window)`);

test("showing, hiding and the narrow breakpoint keep the main list's selection, its filtered selection and its scroll", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 520 });
  await open(page);
  await allTracks(page);
  await mainRow(page, "Bar 02").locator('[data-col="title"]').click();
  await mainRow(page, "Bar 04").locator('[data-col="title"]').click({ modifiers: ["Shift"] });
  await main(page).evaluate((scroller) => { scroller.scrollTop = 120; });
  const scrolled = await main(page).evaluate((scroller) => scroller.scrollTop);
  expect(scrolled).toBeGreaterThan(0);

  await page.keyboard.press("Control+Backslash");
  await expect(target(page)).toBeVisible();
  for (const title of bars(2, 3, 4)) await expect(mainRow(page, title)).toHaveAttribute("aria-selected", "true");
  expect(await main(page).evaluate((scroller) => scroller.scrollTop)).toBe(scrolled);
  await page.keyboard.press("Control+Backslash");
  await expect(target(page)).toHaveCount(0);
  for (const title of bars(2, 3, 4)) await expect(mainRow(page, title)).toHaveAttribute("aria-selected", "true");
  expect(await main(page).evaluate((scroller) => scroller.scrollTop)).toBe(scrolled);

  // Across the breakpoint and back, the second list shown.
  await page.keyboard.press("Control+Backslash");
  await page.setViewportSize({ width: 640, height: 520 });
  await expect(target(page)).toHaveCount(0);
  await page.setViewportSize({ width: 1280, height: 520 });
  await expect(target(page)).toBeVisible();
  for (const title of bars(2, 3, 4)) await expect(mainRow(page, title)).toHaveAttribute("aria-selected", "true");

  // The filter narrows the selection to what it shows; a toggle keeps that,
  // and Enter adds exactly what is selected and shown.
  await picker(page).selectOption({ label: (await picker(page).locator("option").filter({ hasText: /Eurodance$/ }).innerText()).trim() });
  await page.getByRole("search", { name: "Track Filter" }).getByRole("textbox", { name: "Filter text" }).fill("Bar 03");
  await expect(mainRows(page)).toHaveCount(1);
  // The keyboard back to the page, the main list still the browser's focus.
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press("Control+Backslash");
  await page.keyboard.press("Control+Backslash");
  await page.keyboard.press("Enter");
  await expect.poll(() => entryTitles(page, "Eurodance")).toEqual(bars(3));
});

test("Enter in the tree opens the focused playlist, and adds nothing", async ({ page }) => {
  await open(page);
  await allTracks(page);
  await showTarget(page, "Eurodance");
  await mainRow(page, "Bar 02").locator('[data-col="title"]').click();
  await shellPlaylist(page, "Hardstyle").focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("browser-title")).toContainText("Hardstyle");
  await expect.poll(() => targetTitles(page)).toEqual([]);
  expect(await entryTitles(page, "Eurodance")).toEqual([]);
});

test("two reorders asked before the Library answers make the order both describe, by entry, and the selection follows its entry", async ({ page }) => {
  await voxTarget(page);
  const before = await entries(page, "Melodic Vox");
  await hooks(page, (h) => h.__holdEntryReorders());
  await targetRows(page).nth(0).click();
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.press("Alt+ArrowDown");
  // Shown at once, before any write has landed.
  await expect.poll(() => targetTitles(page)).toEqual(bars(4, 5, 3, ...range(6, 20)));
  await expect(targetRows(page).nth(2)).toHaveAttribute("aria-selected", "true");
  await hooks(page, (h) => h.__releaseEntryReorders());
  const [a, b, c] = before;
  await expect.poll(async () => (await entries(page, "Melodic Vox")).map((e) => e.entryId).slice(0, 3)).toEqual([b!.entryId, c!.entryId, a!.entryId]);
  // Each write named every entry, the second on top of the first.
  const asked = await hooks(page, (h) => h.__entryReorders);
  expect(asked.map((order) => order.slice(0, 3))).toEqual([[b!.entryId, a!.entryId, c!.entryId], [b!.entryId, c!.entryId, a!.entryId]]);
  await expect.poll(() => targetTitles(page)).toEqual(bars(4, 5, 3, ...range(6, 20)));
  await expect(targetRows(page).nth(2)).toHaveAttribute("aria-selected", "true");

  // A held key's repeats each move it once more.
  await page.keyboard.down("Alt");
  await page.keyboard.down("ArrowDown");
  await page.keyboard.down("ArrowDown");
  await page.keyboard.down("ArrowDown");
  await page.keyboard.up("ArrowDown");
  await page.keyboard.up("Alt");
  await expect.poll(() => entryTitles(page, "Melodic Vox")).toEqual(bars(4, 5, 6, 7, 8, 3, ...range(9, 20)));
  await expect(targetRows(page).nth(5)).toHaveAttribute("aria-selected", "true");
});

test("a reorder the Library refuses puts back its order, says so, and keeps the selection on its entry", async ({ page }) => {
  await voxTarget(page);
  await targetRows(page).nth(1).click();
  await hooks(page, (h) => { h.__failNextEntryReorder = true; });
  await page.keyboard.press("Alt+ArrowDown");
  await expect(status(page)).toContainText("could not be saved");
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
  await expect(targetRows(page).nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(targetRows(page).nth(1)).toContainText("Bar 04");
  expect(await entryTitles(page, "Melodic Vox")).toEqual(bars(...range(3, 20)));
  // And the next one is written.
  await page.keyboard.press("Alt+ArrowDown");
  await expect.poll(() => entryTitles(page, "Melodic Vox")).toEqual(bars(3, 5, 4, ...range(6, 20)));
});

test("under Library Protection a reorder is refused and the target's order is unchanged", async ({ page }) => {
  await voxTarget(page);
  await hooks(page, (h) => h.__turnLibraryProtection(true));
  await targetRows(page).nth(0).click();
  await page.keyboard.press("Alt+ArrowDown");
  await expect(status(page)).toContainText("Editing is locked by Library Protection");
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
  expect(await entryTitles(page, "Melodic Vox")).toEqual(bars(...range(3, 20)));
  expect(await hooks(page, (h) => h.__entryReorders)).toEqual([]);
});

test("a track added twice is two entries, and moving one moves exactly that entry", async ({ page }) => {
  await open(page);
  await allTracks(page);
  await showTarget(page, "Eurodance");
  const add = async (title: string) => {
    await mainRow(page, title).locator('[data-col="title"]').click();
    await page.keyboard.press("Enter");
  };
  await add("Bar 05");
  await expect.poll(() => entryTitles(page, "Eurodance")).toEqual(bars(5));
  // Again: the duplicate question. Skip adds nothing; Add duplicates adds it.
  await add("Bar 05");
  const question = page.getByRole("alertdialog", { name: "Duplicate tracks" });
  await question.getByRole("button", { name: "Skip duplicates" }).click();
  await expect(status(page)).toContainText("Already in Eurodance.");
  expect(await entryTitles(page, "Eurodance")).toEqual(bars(5));
  await add("Bar 05");
  await question.getByRole("button", { name: "Add duplicates" }).click();
  await expect.poll(() => entryTitles(page, "Eurodance")).toEqual(bars(5, 5));
  await add("Bar 06");
  await expect.poll(() => targetTitles(page)).toEqual(bars(5, 5, 6));
  const [first, second, six] = await entries(page, "Eurodance");
  expect(first!.entryId).not.toBe(second!.entryId);

  // The second entry of Bar 05 to the end: only it moves.
  await targetRows(page).nth(1).click();
  await page.keyboard.press("Alt+ArrowDown");
  await expect.poll(async () => (await entries(page, "Eurodance")).map((e) => e.entryId)).toEqual([first!.entryId, six!.entryId, second!.entryId]);
  await expect(targetRows(page).nth(2)).toHaveAttribute("aria-selected", "true");
  await menu(page, "undo");
  await expect.poll(async () => (await entries(page, "Eurodance")).map((e) => e.entryId)).toEqual([first!.entryId, second!.entryId, six!.entryId]);
  await menu(page, "redo");
  await expect.poll(async () => (await entries(page, "Eurodance")).map((e) => e.entryId)).toEqual([first!.entryId, six!.entryId, second!.entryId]);
});

test("with the target focused, the list's keys move only the target, also when it is empty, and the active row is in view and named", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 520 });
  await voxTarget(page);
  await mainRow(page, "Bar 00").locator('[data-col="title"]').click();
  const selectedInMain = () => mainRows(page).and(page.locator('[aria-selected="true"]')).count();
  await targetRows(page).nth(0).click();
  for (const key of ["PageDown", "End", "Control+a", "Home", "ArrowDown", "PageUp", "Delete"]) {
    await page.keyboard.press(key);
    expect(await selectedInMain()).toBe(1);
    await expect(mainRow(page, "Bar 00")).toHaveAttribute("aria-selected", "true");
  }
  // End: the last row is selected, scrolled into view, and is the grid's active descendant.
  await page.keyboard.press("End");
  const last = targetRows(page).last();
  await expect(last).toHaveAttribute("aria-selected", "true");
  await expect(last).toBeInViewport();
  await expect(targetGrid(page)).toHaveAttribute("aria-activedescendant", (await last.getAttribute("id"))!);
  await page.keyboard.press("PageUp");
  const active = await targetGrid(page).getAttribute("aria-activedescendant");
  await expect(targetGrid(page).locator(`[id="${active}"]`)).toBeInViewport();
  // Nothing was removed from the open list by Delete in the target.
  expect(await entryTitles(page, "Melodic Vox")).toEqual(bars(...range(3, 20)));

  // An empty target keeps the keys too.
  await picker(page).selectOption({ label: (await picker(page).locator("option").filter({ hasText: /Eurodance$/ }).innerText()).trim() });
  await targetGrid(page).click();
  for (const key of ["ArrowDown", "PageDown", "End", "Control+a"]) {
    await page.keyboard.press(key);
    expect(await selectedInMain()).toBe(1);
  }
});

test("the target renamed or deleted by another client: the list follows the new name, then says it is gone", async ({ page }) => {
  await open(page);
  await allTracks(page);
  await showTarget(page, "Melodic Vox");
  await hooks(page, (h) => h.__renamePlaylistOutside("Melodic Vox", "Vox Renamed"));
  await expect(target(page).getByRole("heading")).toContainText("Vox Renamed");
  await expect(shellPlaylist(page, "Vox Renamed")).toHaveAccessibleDescription("Target playlist");
  await hooks(page, (h) => h.__deletePlaylistOutside("Vox Renamed"));
  await expect(target(page).getByRole("status")).toContainText("no longer in the Library");
  await expect(target(page).getByRole("heading")).toContainText("No target");
  await expect(tree(page).locator("[aria-description]")).toHaveCount(0);
  await mainRow(page, "Bar 01").locator('[data-col="title"]').click();
  await page.keyboard.press("Enter");
  await expect(target(page).getByRole("status")).toContainText("Choose a target playlist first.");
});

test("a target that cannot be read again is shown as not current, is not reordered, and reads again on Try again", async ({ page }) => {
  await voxTarget(page);
  await hooks(page, (h) => { h.__failPlaylistEntries = true; });
  // Another client's change makes the list read again; the read fails.
  await hooks(page, (h) => h.__renamePlaylistOutside("Hardstyle", "Hardstyle 2"));
  await expect(target(page).getByRole("alert")).toContainText("Not current");
  await expect.poll(() => targetTitles(page)).toEqual(bars(...range(3, 20)));
  await targetRows(page).nth(0).click();
  await page.keyboard.press("Alt+ArrowDown");
  await expect(target(page).getByRole("status")).toContainText("could not be read");
  expect(await hooks(page, (h) => h.__entryReorders)).toEqual([]);
  await hooks(page, (h) => { h.__failPlaylistEntries = false; });
  await target(page).getByRole("button", { name: "Try again" }).click();
  await expect(target(page).getByRole("alert")).toHaveCount(0);
});

test("a swap clears the target's selection and swaps back to where it started", async ({ page }) => {
  await open(page);
  await openInMain(page, "Melodic Vox");
  await mainRows(page).first().click();
  await page.keyboard.press("Control+Backslash");
  await picker(page).selectOption({ label: (await picker(page).locator("option").filter({ hasText: /Hardstyle$/ }).innerText()).trim() });
  await targetRows(page).nth(1).click();
  await page.keyboard.press("Control+Shift+Backslash");
  await expect(page.getByTestId("browser-title")).toContainText("Hardstyle");
  await expect(target(page).getByRole("heading")).toContainText("Melodic Vox");
  await expect(targetGrid(page).locator('[aria-selected="true"]')).toHaveCount(0);
  await expect(picker(page)).toHaveValue(await shellPlaylist(page, "Melodic Vox").getAttribute("data-row-id") ?? "");
  await page.keyboard.press("Control+Shift+Backslash");
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await expect(target(page).getByRole("heading")).toContainText("Hardstyle");
});

test("holding Enter in the main list adds the selection once, not once per key repeat", async ({ page }) => {
  await open(page);
  await allTracks(page);
  await showTarget(page, "Eurodance");
  await mainRow(page, "Bar 07").locator('[data-col="title"]').click();

  // The first press, then the browser's repeats while the key is held.
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await expect(status(page)).toContainText("Added 1 track to Eurodance.");

  // The next add is made after anything the repeats could have started: a
  // second add of Bar 07 would have opened the duplicate question in its way.
  await mainRow(page, "Bar 08").locator('[data-col="title"]').click();
  await page.keyboard.press("Enter");
  await expect.poll(() => targetTitles(page)).toEqual(bars(7, 8));
  await expect(page.getByRole("alertdialog", { name: "Duplicate tracks" })).toHaveCount(0);
  expect(await playlistAsListed(page, "Eurodance")).toEqual(bars(7, 8));
});
