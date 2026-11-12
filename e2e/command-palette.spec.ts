/**
 * The command palette (ticket #54): every action the window offers, with
 * its key, found by typing and run from one place. On Linux it replaces the
 * native menu row, so every item the menus held is in it, an item that
 * cannot run now is shown greyed with the reason rather than left out, and
 * the parts of Preferences the four views do not show are reachable here.
 *
 * The mock's `runMenuItem` keeps what the shell was asked for in
 * `window.__shellItems` (full screen, Close Window, Quit).
 */
import { expect, test, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const palette = (page: Page) => page.getByRole("dialog", { name: "Command palette" });
const search = (page: Page) => palette(page).getByRole("combobox", { name: "Search actions" });
const results = (page: Page) => palette(page).getByRole("listbox", { name: "Actions" });
const option = (page: Page, name: string | RegExp) => results(page).getByRole("option", { name });
const preferences = (page: Page) => page.getByRole("dialog", { name: "Preferences" });
const rows = (page: Page) => page.getByRole("row").filter({ has: page.getByRole("gridcell") });

async function open(page: Page, query = "") {
  await page.goto(`/${query}`);
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
}

async function openPalette(page: Page) {
  await page.keyboard.press("Control+Shift+P");
  await expect(palette(page)).toBeVisible();
  await expect(search(page)).toBeFocused();
}

test("the palette opens on its key, lists every action with its key, and closes on Escape", async ({ page }) => {
  await open(page);
  await openPalette(page);
  // Native menu items, with the keys they had.
  await expect(option(page, /^Import Folder…/)).toContainText("shift + ctrl + O");
  await expect(option(page, /^Preferences/)).toContainText("ctrl + ,");
  await expect(option(page, /^Locate track loaded on deck/)).toContainText("ctrl + L");
  // An action with no key shows none.
  await expect(option(page, /^Backups/)).not.toContainText("ctrl");
  expect(await results(page).getByRole("option").count()).toBeGreaterThan(40);

  await page.keyboard.press("Escape");
  await expect(palette(page)).toHaveCount(0);
});

test("typing filters the list and Enter runs the highlighted action", async ({ page }) => {
  await open(page);
  await openPalette(page);
  await search(page).fill("2 play");
  await expect(results(page).getByRole("option")).toHaveCount(1);
  await expect(option(page, /^2 Players/)).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Enter");
  await expect(palette(page)).toHaveCount(0);
  // The two-deck layout: Player B is drawn.
  await expect(page.getByRole("region", { name: "Preview player B" })).toBeVisible();

  await openPalette(page);
  await search(page).fill("zzzz no such action");
  await expect(results(page).getByRole("option")).toHaveCount(0);
  await expect(palette(page)).toContainText("No action matches");
});

test("the arrow keys move the highlight and a click runs an action", async ({ page }) => {
  await open(page);
  await openPalette(page);
  await search(page).fill("player");
  await expect(results(page).getByRole("option").first()).toHaveAttribute("aria-selected", "true");
  await expect(results(page).getByRole("option").nth(1)).toHaveAttribute("aria-selected", "false");
  await page.keyboard.press("ArrowDown");
  await expect(results(page).getByRole("option").nth(1)).toHaveAttribute("aria-selected", "true");

  await search(page).fill("backups");
  await option(page, /^Backups/).click();
  await expect(preferences(page).getByRole("tab", { name: "Backups" })).toHaveAttribute("aria-selected", "true");
});

test("Preferences, PRO DJ LINK, device settings and backups open from the palette", async ({ page }) => {
  await open(page);
  for (const [query, pane] of [
    ["preferences", "View"],
    ["pro dj link", "PRO DJ LINK"],
    ["device settings", "DJ System"],
    ["backups", "Backups"],
  ] as const) {
    await openPalette(page);
    await search(page).fill(query);
    await page.keyboard.press("Enter");
    await expect(preferences(page).getByRole("tab", { name: pane, exact: true })).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Escape");
    await expect(preferences(page)).toHaveCount(0);
  }
});

test("an action that cannot run now is shown greyed with the reason, and Enter does nothing", async ({ page }) => {
  await open(page, "?writable=1");
  await openPalette(page);
  await search(page).fill("remove from collection");
  const remove = option(page, /^Remove from Collection/);
  await expect(remove).toHaveAttribute("aria-disabled", "true");
  await expect(remove).toContainText("Select tracks in a list first");
  await page.keyboard.press("Enter");
  // Still open: nothing ran.
  await expect(palette(page)).toBeVisible();
  await page.keyboard.press("Escape");

  // With a track selected in the list it is live, and runs the same command
  // the Track menu ran.
  await rows(page).nth(1).click();
  await openPalette(page);
  await search(page).fill("remove from collection");
  await expect(option(page, /^Remove from Collection/)).not.toHaveAttribute("aria-disabled", "true");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("contentinfo")).toContainText("Removed 1 track from the collection.");
});

test("the keys the menu answered still work: full screen goes to the shell", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Control+Shift+F");
  await expect.poll(() => page.evaluate(() => (window as unknown as { __shellItems?: string[] }).__shellItems ?? []))
    .toEqual(["fullscreen"]);
});

test("a button in the old layout's top bar opens the palette", async ({ page }) => {
  await open(page);
  await page.getByRole("banner").getByRole("button", { name: "Command palette" }).click();
  await expect(palette(page)).toBeVisible();
});

test.describe("with the view shell", () => {
  test("its top bar opens the palette, which shows a view and greys what the shell does not draw", async ({ page }) => {
    await enableViewShell(page);
    await open(page);
    const opener = page.getByTestId("shell-top-bar-end").getByRole("button", { name: "Command palette" });
    await expect(opener).toContainText("shift + ctrl + P");
    await opener.click();
    await expect(palette(page)).toBeVisible();

    await search(page).fill("2 players");
    await expect(option(page, /^2 Players/)).toHaveAttribute("aria-disabled", "true");
    await expect(option(page, /^2 Players/)).toContainText("Not in the view shell yet");

    await search(page).fill("intake");
    await expect(option(page, /^Show Intake/)).toContainText("2");
    await page.keyboard.press("Enter");
    await expect(page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "Intake" }))
      .toHaveAttribute("aria-selected", "true");
  });

  test("the view keys do nothing while the palette has the keyboard", async ({ page }) => {
    await enableViewShell(page);
    await open(page);
    await openPalette(page);
    await page.keyboard.type("3");
    await expect(search(page)).toHaveValue("3");
    await expect(page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "Library" }))
      .toHaveAttribute("aria-selected", "true");
  });
});
