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
import { load as loadCompact, player as compactPlayer, titles, elapsedSeconds } from "./compactPlayer";

const palette = (page: Page) => page.getByRole("dialog", { name: "Command palette" });
const search = (page: Page) => palette(page).getByRole("combobox", { name: "Search actions" });
const results = (page: Page) => palette(page).getByRole("listbox", { name: "Actions" });
const option = (page: Page, name: string | RegExp) => results(page).getByRole("option", { name });
const preferences = (page: Page) => page.getByRole("dialog", { name: "Preferences" });
const rows = (page: Page) => page.getByRole("row").filter({ has: page.getByRole("gridcell") });
const status = (page: Page) => page.getByRole("contentinfo");
const treeItem = (page: Page, name: string) => page.getByRole("treeitem").filter({ hasText: name }).first();
const searchField = (page: Page) => page.getByRole("searchbox", { name: /search within/i });

/** Opens the palette, finds an entry and runs it with Enter. */
async function runEntry(page: Page, query: string, name: RegExp) {
  await openPalette(page);
  await search(page).fill(query);
  await expect(results(page).getByRole("option").first()).toHaveAccessibleName(name);
  await page.keyboard.press("Enter");
  await expect(palette(page)).toHaveCount(0);
}

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
  await expect(results(page).getByRole("option").first()).toHaveAccessibleName(/^2 Players/);
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

test.describe("with the view shell, the old layout's parts", () => {
  test("2 Players switches to the two-deck layout, and Use the view shell comes back", async ({ page }) => {
    await enableViewShell(page);
    await open(page);
    await expect(page.getByTestId("shell")).toBeVisible();
    await openPalette(page);
    await search(page).fill("2 players");
    await expect(option(page, /^2 Players/)).not.toHaveAttribute("aria-disabled", "true");
    await expect(option(page, /^2 Players/)).toContainText("Switches to the old layout");
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("shell")).toHaveCount(0);
    await expect(page.getByRole("region", { name: "Preview player B" })).toBeVisible();

    await runEntry(page, "use the view shell", /^Use the view shell/);
    await expect(page.getByTestId("shell")).toBeVisible();
    await expect(page.getByRole("region", { name: "Preview player B" })).toHaveCount(0);
  });

  test("the Information window opens in the old layout", async ({ page }) => {
    await enableViewShell(page);
    await open(page);
    await runEntry(page, "information window", /^Information Window/);
    await expect(page.getByTestId("shell")).toHaveCount(0);
    await expect(page.getByRole("complementary", { name: "Information" })).toBeVisible();
  });
});

test("Undo from the palette undoes the Library edit, not the palette's search text (after a click on a row)", async ({ page }) => {
  await open(page, "?writable=1");
  await treeItem(page, "Melodic Vox").click();
  await runEntry(page, "create new playlist", /^Create New Playlist/);
  await expect(treeItem(page, "New playlist")).toBeVisible();

  // A click on a row leaves no element with the keyboard (the page has it).
  await rows(page).nth(1).click();
  await runEntry(page, "undo", /^Undo/);
  await expect(status(page)).toContainText("Edit undone.");
  await expect(page.getByRole("treeitem").filter({ hasText: "New playlist" })).toHaveCount(0);

  await runEntry(page, "redo", /^Redo/);
  await expect(status(page)).toContainText("Edit redone.");
  await expect(treeItem(page, "New playlist")).toBeVisible();
});

test("Undo and Redo are greyed with nothing to undo or redo", async ({ page }) => {
  await open(page, "?writable=1");
  await openPalette(page);
  await search(page).fill("undo");
  await expect(option(page, /^Undo/)).toHaveAttribute("aria-disabled", "true");
  await expect(option(page, /^Undo/)).toContainText("Nothing to undo");
});

test("Paste puts the clipboard into the field that had the keyboard, over its selection", async ({ page }) => {
  await open(page);
  await page.evaluate(() => { (window as unknown as { __clipboard: string }).__clipboard = "Vox"; });
  await searchField(page).fill("abcdef");
  // Select "cd".
  await searchField(page).evaluate((field: HTMLInputElement) => field.setSelectionRange(2, 4));
  await runEntry(page, "paste", /^Paste/);
  await expect(searchField(page)).toHaveValue("abVoxef");
  await expect(searchField(page)).toBeFocused();
  // Through the webview's own editing action, not the page's clipboard.
  expect(await page.evaluate(() => (window as unknown as { __editCommands?: string[] }).__editCommands)).toEqual(["paste"]);

  // Copy and Cut take the selection; Cut removes it.
  await searchField(page).evaluate((field: HTMLInputElement) => field.setSelectionRange(0, 2));
  await runEntry(page, "cut", /^Cut/);
  await expect(searchField(page)).toHaveValue("Voxef");
  expect(await page.evaluate(() => (window as unknown as { __clipboard?: string }).__clipboard)).toBe("ab");
});

test.describe("while a modal dialog owns the keyboard the palette does not open", () => {
  test("a native modal <dialog>: Analyze Track", async ({ page }) => {
    await open(page, "?writable=1");
    await rows(page).nth(1).click();
    await page.keyboard.press("Control+Shift+A");
    const analyze = page.getByRole("dialog", { name: /analy/i });
    await expect(analyze).toBeVisible();
    await page.keyboard.press("Control+Shift+P");
    await page.waitForTimeout(200);
    await expect(page.getByTestId("command-palette")).toHaveCount(0);
    await expect(analyze).toBeVisible();
  });

  test("an ARIA alertdialog", async ({ page }) => {
    await open(page);
    await page.evaluate(() => {
      const prompt = document.createElement("div");
      prompt.setAttribute("role", "alertdialog");
      prompt.setAttribute("aria-label", "Confirm");
      prompt.innerHTML = '<button type="button">OK</button>';
      document.body.append(prompt);
      prompt.querySelector("button")?.focus();
    });
    await page.keyboard.press("Control+Shift+P");
    await page.waitForTimeout(200);
    await expect(page.getByTestId("command-palette")).toHaveCount(0);
  });

  test("but it does open from a text field", async ({ page }) => {
    await open(page);
    await searchField(page).focus();
    await openPalette(page);
  });
});

test("Library Protection greys the Track and Playlist edits", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({ advanced: { protectLibrary: true } })));
  await open(page, "?writable=1");
  await rows(page).nth(1).click();
  await openPalette(page);
  for (const name of ["Remove from Collection", "Relocate", "Analyze Track"]) {
    await search(page).fill(name.toLowerCase());
    await expect(option(page, new RegExp(`^${name}`))).toHaveAttribute("aria-disabled", "true");
    await expect(option(page, new RegExp(`^${name}`))).toContainText("The Library is locked for editing");
  }
  await search(page).fill("show in finder");
  await expect(option(page, /^Show in Finder/)).not.toHaveAttribute("aria-disabled", "true");
  await page.keyboard.press("Escape");
  await treeItem(page, "Melodic Vox").click();
  await openPalette(page);
  await search(page).fill("delete playlist");
  await expect(option(page, /^Delete Playlist/)).toContainText("The Library is locked for editing");
});

test("cancelling gives the list back its keyboard: Track commands stay live after Escape", async ({ page }) => {
  await open(page, "?writable=1");
  await rows(page).nth(1).click();
  await openPalette(page);
  // A click in the palette's own field.
  await search(page).click();
  await page.keyboard.press("Escape");
  await expect(palette(page)).toHaveCount(0);
  await openPalette(page);
  await search(page).fill("remove from collection");
  await expect(option(page, /^Remove from Collection/)).not.toHaveAttribute("aria-disabled", "true");
  // The backdrop dismisses the same way.
  await page.getByTestId("command-palette").click({ position: { x: 5, y: 5 } });
  await expect(palette(page)).toHaveCount(0);
  await openPalette(page);
  await search(page).fill("remove from collection");
  await expect(option(page, /^Remove from Collection/)).not.toHaveAttribute("aria-disabled", "true");
});

test.describe("the window's own actions are in the palette", () => {
  test("Play/Pause and Cue on the loaded deck, greyed while the deck is empty", async ({ page }) => {
    await enableViewShell(page);
    await open(page);
    await openPalette(page);
    await search(page).fill("play/pause");
    await expect(results(page).getByRole("option").first()).toHaveAccessibleName(/^Play\/Pause.*Player A/);
    await expect(results(page).getByRole("option").first()).toHaveAttribute("aria-disabled", "true");
    await expect(results(page).getByRole("option").first()).toContainText("Load a track on Player A first");
    await page.keyboard.press("Escape");

    await titles(page).nth(3).dblclick();
    await expect(compactPlayer(page).getByRole("button", { name: "Play", exact: true })).toBeEnabled();
    await runEntry(page, "play/pause", /^Play\/Pause/);
    await expect(compactPlayer(page).getByRole("button", { name: "Pause", exact: true })).toBeVisible();
    await runEntry(page, "play/pause", /^Play\/Pause/);
    await expect(compactPlayer(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();
  });

  test("Mute and Volume Down move the master level", async ({ page }) => {
    await open(page);
    const knob = page.getByRole("slider", { name: "Master level" });
    const before = Number(await knob.getAttribute("aria-valuenow"));
    expect(before).toBeGreaterThan(0);
    await runEntry(page, "mute", /^Mute/);
    await expect(knob).toHaveAttribute("aria-valuenow", "0");
    await runEntry(page, "mute", /^Mute/);
    await expect(knob).toHaveAttribute("aria-valuenow", String(before));
    await runEntry(page, "volume down", /^Volume Down/);
    await expect.poll(async () => Number(await knob.getAttribute("aria-valuenow"))).toBeLessThan(before);
  });
});

test("the compact player keeps the arrows after the palette closes", async ({ page }) => {
  await loadCompact(page);
  await compactPlayer(page).getByTestId("compact-player-title").click();
  await expect(compactPlayer(page)).toHaveAttribute("data-armed", "");
  await openPalette(page);
  await search(page).click();
  await page.keyboard.press("Escape");
  await expect(palette(page)).toHaveCount(0);
  await expect(compactPlayer(page)).toHaveAttribute("data-armed", "");
  const start = await elapsedSeconds(page);
  await page.keyboard.press("ArrowRight");
  await expect.poll(() => elapsedSeconds(page)).toBeGreaterThan(start);
});
