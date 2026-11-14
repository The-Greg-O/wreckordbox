/**
 * Saving the Track Filter as an intelligent playlist (wbx-58).
 *
 * The bar's "Save as intelligent playlist" names the filter and saves it;
 * the new intelligent playlist appears in the tree and lists the tracks
 * the filter showed. A filter an intelligent playlist cannot hold is
 * refused with the reason, and nothing appears. Saving is one Undo step;
 * a protected Library refuses it.
 *
 * On the mock's constructed Library (`?filterFixture=1`, see
 * `track-filter-bar.spec.ts` for its definition): track i of 0 to 29 is
 * "Bar ii", Situation Peak when i is even, Energy (i / 2) mod 6, Prep colour
 * i mod 9 (5 Green, 2 Red). The backend's own rule and refusals are tested
 * in `src-tauri/tests/it/save_filter_playlist.rs`.
 */
import { expect, test, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const working = (page: Page) => page.getByRole("tabpanel");
const bar = (page: Page) => page.getByRole("search", { name: "Track Filter" });
const grid = (page: Page) => working(page).getByRole("grid");
const rows = (page: Page) => working(page).locator('[role="row"][aria-selected]');
const count = (page: Page) => bar(page).getByRole("status").first();
const tree = (page: Page) => page.getByTestId("shell-tree");
const shellPlaylist = (page: Page, name: string) =>
  tree(page).getByRole("group", { name: "Playlists" }).getByRole("treeitem", { name, exact: true });
const saveButton = (page: Page) => bar(page).getByRole("button", { name: "Save as intelligent playlist" });
const form = (page: Page) => bar(page).getByRole("form", { name: "Save the filter as an intelligent playlist" });
const nameField = (page: Page) => form(page).getByRole("textbox", { name: "Intelligent playlist name" });
const menu = (page: Page, id: string) =>
  page.evaluate((item) => (window as unknown as { __menu?: (id: string) => void }).__menu?.(item), id);

const bars = (...ids: number[]) => ids.map((i) => `Bar ${String(i).padStart(2, "0")}`);

async function allTracks(page: Page) {
  await tree(page).getByRole("treeitem", { name: /^All Tracks/ }).click();
  await expect(page.getByTestId("browser-title")).toContainText("All Tracks");
  await expect(rows(page).first()).toBeVisible();
}

async function open(page: Page) {
  await enableViewShell(page);
  await page.goto("/?writable=1&filterFixture=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await allTracks(page);
}

async function titles(page: Page, length: number): Promise<string[]> {
  await expect.poll(async () => Number(await grid(page).getAttribute("aria-rowcount"))).toBe(length);
  await expect(rows(page)).toHaveCount(length);
  return (await rows(page).locator('[role="gridcell"][data-col="title"]').allInnerTexts()).map((t) => t.trim());
}

async function pick(page: Page, controlName: string, groupName: string, choice: string) {
  const control = bar(page).getByRole("button", { name: new RegExp(`^${controlName}( .*)?$`) }).first();
  if (await control.getAttribute("aria-expanded") !== "true") await control.click();
  await bar(page).getByRole("group", { name: groupName, exact: true }).getByRole("button", { name: choice, exact: true }).click();
}

async function energy(page: Page, from: string, to: string) {
  const control = bar(page).getByRole("button", { name: /^Energy( .*)?$/ });
  if (await control.getAttribute("aria-expanded") !== "true") await control.click();
  await bar(page).getByRole("combobox", { name: "Energy from" }).selectOption(from);
  await bar(page).getByRole("combobox", { name: "Energy to" }).selectOption(to);
}

/** Situation Peak, Energy 3 to 5: Bar 06, 08, 10, 18, 20, 22. */
async function peakAndEnergy(page: Page) {
  await pick(page, "Situation", "Situation tags", "Peak");
  await energy(page, "3", "5");
  await expect(count(page)).toHaveText("6 of 30 tracks");
}

async function saveAs(page: Page, name: string) {
  await saveButton(page).click();
  await expect(form(page)).toBeVisible();
  await nameField(page).fill(name);
  await form(page).getByRole("button", { name: "Save", exact: true }).click();
}

test("the filter is saved under the name given and the new intelligent playlist lists the tracks it showed", async ({ page }) => {
  await open(page);
  await peakAndEnergy(page);
  const shown = await titles(page, 6);
  expect(shown).toEqual(bars(6, 8, 10, 18, 20, 22));

  await saveButton(page).click();
  // The name it suggests says what is picked.
  await expect(nameField(page)).toHaveValue("Peak · Energy 3-5");
  await nameField(page).fill("Peak hour");
  await form(page).getByRole("button", { name: "Save", exact: true }).click();
  await expect(form(page)).toHaveCount(0);
  await expect(bar(page).getByRole("status").filter({ hasText: "Saved “Peak hour”" })).toBeVisible();
  await expect(shellPlaylist(page, "Peak hour")).toBeVisible();

  // With the filter cleared, the playlist alone lists the same tracks.
  await bar(page).getByRole("button", { name: "Clear filter" }).click();
  await expect(count(page)).toHaveText("30 tracks");
  await shellPlaylist(page, "Peak hour").click();
  await expect(page.getByTestId("browser-title")).toContainText("Peak hour");
  expect(await titles(page, 6)).toEqual(shown);
});

test("saving is one Undo step", async ({ page }) => {
  await open(page);
  await peakAndEnergy(page);
  await saveAs(page, "Peak hour");
  await expect(shellPlaylist(page, "Peak hour")).toBeVisible();
  await menu(page, "undo");
  await expect(shellPlaylist(page, "Peak hour")).toHaveCount(0);
  await menu(page, "redo");
  await expect(shellPlaylist(page, "Peak hour")).toBeVisible();
});

test("a filter an intelligent playlist cannot hold is refused with the reason, and nothing is saved", async ({ page }) => {
  await open(page);
  const before = await tree(page).getByRole("treeitem").count();

  // Text: no rule searches every word everywhere.
  await bar(page).getByRole("textbox", { name: "Filter text" }).fill("Bar 1");
  await saveAs(page, "Text");
  await expect(form(page).getByRole("alert")).toHaveText(
    "Not saved: An intelligent playlist cannot search for text, so it cannot keep \"Bar 1\". Clear the text to save this filter.",
  );
  await expect(form(page)).toBeVisible();
  await form(page).getByRole("button", { name: "Cancel" }).click();
  await bar(page).getByRole("button", { name: "Clear filter" }).click();

  // Two Prep colours (an OR) beside Energy (an AND).
  await pick(page, "Prep colour", "Prep colours", "Green");
  await pick(page, "Prep colour", "Prep colours", "Red");
  await energy(page, "2", "4");
  await saveAs(page, "Mixed");
  await expect(form(page).getByRole("alert")).toContainText(
    "cannot hold 2 Prep colours (Green, Red) together with the rest of this filter",
  );
  await form(page).getByRole("button", { name: "Cancel" }).click();

  // On a playlist: an intelligent playlist chooses from the whole Library.
  await shellPlaylist(page, "Melodic Vox").click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await saveAs(page, "Within");
  await expect(form(page).getByRole("alert")).toContainText("cannot keep this filter to \"Melodic Vox\". Open All Tracks to save it.");

  await expect(tree(page).getByRole("treeitem")).toHaveCount(before);
});

test("with nothing picked there is nothing to save", async ({ page }) => {
  await open(page);
  await expect(saveButton(page)).toBeDisabled();
  await pick(page, "Prep colour", "Prep colours", "Green");
  await expect(saveButton(page)).toBeEnabled();
});

test("under Library Protection the save is refused, and nothing is saved", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
    advanced: { protectLibrary: true }, view: { viewShell: true },
  })));
  await page.goto("/?writable=1&filterFixture=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await allTracks(page);
  await peakAndEnergy(page);
  await saveAs(page, "Protected");
  await expect(form(page).getByRole("alert")).toHaveText(
    "Not saved: Editing is locked by Library Protection. Turn it off in Preferences to edit.",
  );
  await expect(shellPlaylist(page, "Protected")).toHaveCount(0);
});

test("the save works from the keyboard, and Space and Enter never reach the deck", async ({ page }) => {
  await open(page);
  const deck = page.getByTestId("shell-player");
  await rows(page).nth(3).locator('[data-col="title"]').dblclick();
  await expect(deck.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  await peakAndEnergy(page);

  await saveButton(page).focus();
  await page.keyboard.press("Space");
  await expect(nameField(page)).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(form(page)).toHaveCount(0);
  await expect(saveButton(page)).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(nameField(page)).toBeFocused();
  await page.keyboard.press("Control+a");
  await page.keyboard.type("Keyed in");
  await page.keyboard.press("Enter");
  await expect(shellPlaylist(page, "Keyed in")).toBeVisible();
  await expect(deck.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  await expect(deck.getByRole("button", { name: "Pause", exact: true })).toHaveCount(0);
});
