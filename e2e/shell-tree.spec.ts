/**
 * The shell's tree (ticket #52): everything the DJ can open, in one tree, at
 * full width in every view.
 *
 * Against the mock: its Library (All Tracks, three folders, one of them
 * empty, an intelligent playlist) and its two mounted Sticks, DJ STICK (never
 * written, so no library) and TEST (both libraries holding the real stick's
 * three playlists).
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const tree = (page: Page) => page.getByTestId("shell-tree").getByRole("tree", { name: "Tree" });
const group = (page: Page, name: string) => tree(page).getByRole("group", { name, exact: true });
/** The names of a group's items, top to bottom, without their counts. */
const names = async (where: Locator) =>
  (await where.getByRole("treeitem").locator("[data-name]").allTextContents()).map((text) => text.trim());
/** An item by name, in the Library's groups: the Library, Playlists and Set plans. */
const item = (page: Page, name: string) =>
  tree(page).getByRole("group").filter({ hasNot: page.locator('[data-kind="stick"]') }).getByRole("treeitem", { name, exact: true });
const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const working = (page: Page) => page.getByTestId("shell-working");
const rows = (page: Page) => working(page).getByRole("row").filter({ has: page.getByRole("gridcell") });

async function open(page: Page) {
  await enableViewShell(page);
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
}

test("the tree lists the Library's All Tracks, Intake and Problems, every playlist folder with what is in it, and Set plans", async ({ page }) => {
  await open(page);
  await expect(tree(page)).toBeVisible();

  expect(await names(group(page, "Library"))).toEqual(["All Tracks", "Intake", "Problems"]);

  const playlists = group(page, "Playlists");
  const listed = await names(playlists);
  // Every folder, open whatever the old tree left closed, with its playlists
  // and intelligent playlists under it.
  expect(listed.slice(0, 5)).toEqual(["CURRENT", "Melodic Vox", "Hardstyle", "Drum and Bass", "Fresh 128s"]);
  expect(listed).toContain("USB");
  expect(listed).toContain("TRIODE");
  expect(listed.at(-1)).toBe("DOWNLOADS");
  await expect(playlists.getByRole("treeitem", { name: "CURRENT" })).toHaveAttribute("aria-level", "1");
  await expect(playlists.getByRole("treeitem", { name: "Melodic Vox" })).toHaveAttribute("aria-level", "2");
  await expect(playlists.getByRole("treeitem", { name: "Fresh 128s" })).toHaveAttribute("data-kind", "smartPlaylist");
  // Nothing from Histories or the Explorer: those are not places this tree opens.
  expect(listed).not.toContain("2026");

  // No Set plans exist yet: the group says so.
  const plans = group(page, "Set plans");
  await expect(plans).toBeVisible();
  await expect(plans.getByRole("treeitem")).toHaveCount(0);
  await expect(plans).toContainText("No Set plans yet");
});

test("a folder closes and opens, hiding and showing what is in it", async ({ page }) => {
  await open(page);
  const current = item(page, "CURRENT");
  await expect(current).toHaveAttribute("aria-expanded", "true");
  await tree(page).getByRole("button", { name: "Collapse CURRENT" }).click();
  await expect(current).toHaveAttribute("aria-expanded", "false");
  await expect(item(page, "Melodic Vox")).toHaveCount(0);
  await expect(item(page, "USB")).toBeVisible();
  await tree(page).getByRole("button", { name: "Expand CURRENT" }).click();
  await expect(item(page, "Melodic Vox")).toBeVisible();
});

test("selecting a playlist or All Tracks opens it in the Library view, from whichever view is showing", async ({ page }) => {
  await open(page);
  await item(page, "Melodic Vox").click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await expect(item(page, "Melodic Vox")).toHaveAttribute("aria-selected", "true");

  await tab(page, "Review").click();
  await expect(working(page).getByRole("grid")).toHaveCount(0);
  await item(page, "All Tracks").click();
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("browser-title")).toContainText("All Tracks");
  await expect(item(page, "Melodic Vox")).toHaveAttribute("aria-selected", "false");

  await item(page, "Fresh 128s").click();
  await expect(page.getByTestId("browser-title")).toContainText("Fresh 128s");
});

test("selecting Intake or Problems switches to the Intake view on that queue", async ({ page }) => {
  await open(page);
  await item(page, "Intake").click();
  await expect(tab(page, "Intake")).toHaveAttribute("aria-selected", "true");
  await expect(working(page).getByRole("heading", { name: "Intake" })).toBeVisible();
  await expect(item(page, "Intake")).toHaveAttribute("aria-selected", "true");

  await item(page, "Problems").click();
  await expect(tab(page, "Intake")).toHaveAttribute("aria-selected", "true");
  await expect(working(page).getByRole("heading", { name: "Problems" })).toBeVisible();
  await expect(item(page, "Problems")).toHaveAttribute("aria-selected", "true");
  await expect(item(page, "Intake")).toHaveAttribute("aria-selected", "false");

  // The Intake tab, or its key, shows the queue last chosen.
  await tab(page, "Library").click();
  await page.keyboard.press("2");
  await expect(working(page).getByRole("heading", { name: "Problems" })).toBeVisible();

  // A view switch by key leaves the tree's selection where it was: with a
  // playlist chosen last, the playlist stays selected in the Intake view.
  await item(page, "Melodic Vox").click();
  await page.keyboard.press("2");
  await expect(working(page).getByRole("heading", { name: "Problems" })).toBeVisible();
  await expect(item(page, "Melodic Vox")).toHaveAttribute("aria-selected", "true");
  await expect(item(page, "Problems")).toHaveAttribute("aria-selected", "false");
});

test("a mounted Stick appears with its playlists; selecting it opens the Stick view, and one of its playlists the Library view", async ({ page }) => {
  await open(page);
  const stick = group(page, "Stick TEST");
  await expect(stick).toBeVisible();
  // The Stick's libraries are read without opening anything, and its
  // playlists listed once although both of its libraries hold them.
  await expect.poll(() => names(stick)).toEqual(["TEST", "NP3-TEST-MP3", "Melodic Vox", "Now Playing Test"]);
  // A Stick with no library on it yet says so.
  await expect(group(page, "Stick DJ STICK")).toContainText("No playlists");

  await stick.getByRole("treeitem", { name: "TEST", exact: true }).click();
  await expect(tab(page, "Stick")).toHaveAttribute("aria-selected", "true");
  await expect(working(page).getByRole("heading", { name: "Stick" })).toHaveCount(0);
  // Until the Stick view is built, it holds the Stick's existing panel.
  await expect(working(page)).toContainText("77 tracks in 3 playlists");
  await expect(stick.getByRole("treeitem", { name: "TEST", exact: true })).toHaveAttribute("aria-selected", "true");

  await stick.getByRole("treeitem", { name: "Melodic Vox" }).click();
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await expect(rows(page)).toHaveCount(3);
});

test("a Stick that is unmounted leaves the tree", async ({ page }) => {
  await open(page);
  await expect(group(page, "Stick TEST")).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as { __unmountDevice: (path: string) => void }).__unmountDevice("/Volumes/TEST");
    // The window asks for the mounted devices again when it regains focus.
    window.dispatchEvent(new Event("focus"));
  });
  await expect(group(page, "Stick TEST")).toHaveCount(0);
  await expect(group(page, "Stick DJ STICK")).toBeVisible();
});

test("the tree is worked from the keyboard: arrows move, Enter opens, Left and Right close and open a folder", async ({ page }) => {
  await open(page);
  await item(page, "All Tracks").click();
  await expect(item(page, "All Tracks")).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(item(page, "Intake")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(tab(page, "Intake")).toHaveAttribute("aria-selected", "true");

  await item(page, "CURRENT").focus();
  await page.keyboard.press("ArrowLeft");
  await expect(item(page, "CURRENT")).toHaveAttribute("aria-expanded", "false");
  await page.keyboard.press("ArrowDown");
  await expect(item(page, "USB")).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowRight");
  await expect(item(page, "Melodic Vox")).toBeVisible();
});

const TEST = "/Volumes/TEST";
/** Calls one of the mock's hooks for the shell tree's Stick reads. */
async function mock(page: Page, hook: string, ...args: unknown[]) {
  await page.evaluate(({ hook, args }) => {
    (window as unknown as Record<string, (...a: unknown[]) => unknown>)[hook]?.(...args);
  }, { hook, args });
}
/** The window asks for the mounted devices again when it regains focus. */
const refocus = (page: Page) => page.evaluate(() => window.dispatchEvent(new Event("focus")));
const reads = (page: Page) => page.evaluate((path) =>
  (window as unknown as { __stickOutlineReads: (p: string) => number }).__stickOutlineReads(path), TEST);

test("a Stick is read once when it is mounted, and not again when nothing about it changed", async ({ page }) => {
  await open(page);
  await expect.poll(() => names(group(page, "Stick TEST"))).toContain("Melodic Vox");
  expect(await reads(page)).toBe(1);
  for (let i = 0; i < 3; i += 1) await refocus(page);
  await page.waitForTimeout(500);
  expect(await reads(page)).toBe(1);
  // Put back after being pulled out, it is a new mount and read again.
  await mock(page, "__unmountDevice", TEST);
  await refocus(page);
  await expect(group(page, "Stick TEST")).toHaveCount(0);
  await mock(page, "__remountDevice", TEST);
  await refocus(page);
  await expect.poll(() => names(group(page, "Stick TEST"))).toContain("Melodic Vox");
  expect(await reads(page)).toBe(2);
});

test("an answer for a Stick pulled out, or replaced, while it was being read is dropped", async ({ page }) => {
  await open(page);
  await expect.poll(() => names(group(page, "Stick TEST"))).toContain("Melodic Vox");

  // Pulled out while being read: its answer comes too late to bring it back.
  await mock(page, "__holdStickOutline", TEST);
  await mock(page, "__unmountDevice", TEST);
  await refocus(page);
  await expect(group(page, "Stick TEST")).toHaveCount(0);
  await mock(page, "__remountDevice", TEST);
  await refocus(page);
  await expect(group(page, "Stick TEST")).toBeVisible();
  await mock(page, "__unmountDevice", TEST);
  await refocus(page);
  await expect(group(page, "Stick TEST")).toHaveCount(0);
  await mock(page, "__releaseStickOutline", TEST);
  await page.waitForTimeout(500);
  await expect(group(page, "Stick TEST")).toHaveCount(0);

  // Replaced at the same mount point while the old one's read is held: the
  // new Stick's playlists stay, whenever the old answer arrives.
  await mock(page, "__holdStickOutline", TEST);
  await mock(page, "__remountDevice", TEST);
  await refocus(page);
  await expect(group(page, "Stick TEST")).toBeVisible();
  await mock(page, "__unmountDevice", TEST);
  await refocus(page);
  await expect(group(page, "Stick TEST")).toHaveCount(0);
  await mock(page, "__remountDevice", TEST, {
    pending: false,
    libraries: [{ format: "deviceLibrary", nodes: [{ id: "1", parentId: "0", name: "Fresh", folder: false, depth: 0, count: 2 }] }],
  });
  await refocus(page);
  await expect.poll(() => names(group(page, "Stick TEST"))).toEqual(["TEST", "Fresh"]);
  await mock(page, "__releaseStickOutline", TEST);
  await page.waitForTimeout(500);
  expect(await names(group(page, "Stick TEST"))).toEqual(["TEST", "Fresh"]);
});

test("the tree's keys move only the tree: playback and the list's selection are untouched", async ({ page }) => {
  await open(page);
  await item(page, "Melodic Vox").click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  const titles = working(page).locator('[role="gridcell"][data-col="title"]');
  await titles.nth(1).dblclick();
  const play = page.getByTestId("shell-player").getByRole("button", { name: "Play", exact: true });
  await expect(play).toBeEnabled();
  await play.click();
  await expect(page.getByTestId("shell-player").getByRole("button", { name: "Pause", exact: true })).toBeVisible();
  await rows(page).nth(3).click();
  const selected = rows(page).and(page.locator("[data-selected]"));
  await expect(selected).toHaveCount(1);
  const selectedTitle = await selected.locator('[data-col="title"]').innerText();
  const deck = () => page.evaluate(() => (window as unknown as { __deckSeconds: () => { aNow: number } }).__deckSeconds().aNow);

  await item(page, "Melodic Vox").focus();
  const before = await deck();
  // Left goes to the parent folder, Right on an open folder into it, Home and
  // End to either end; each again where it has nowhere further to go.
  await page.keyboard.press("ArrowLeft");
  await expect(item(page, "CURRENT")).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect(item(page, "CURRENT")).toHaveAttribute("aria-expanded", "false");
  await page.keyboard.press("ArrowLeft");
  await expect(item(page, "CURRENT")).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await expect(item(page, "Melodic Vox")).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Home");
  await expect(item(page, "All Tracks")).toBeFocused();
  await page.keyboard.press("Home");
  await page.keyboard.press("End");
  await expect(group(page, "Stick TEST").getByRole("treeitem").last()).toBeFocused();
  await page.keyboard.press("End");
  const after = await deck();

  // The deck ran on without a jump either way.
  expect(after).toBeGreaterThanOrEqual(before);
  expect(after - before).toBeLessThan(4);
  await expect(page.getByTestId("shell-player").getByRole("button", { name: "Pause", exact: true })).toBeVisible();
  // The list's selection did not move, and the tree's selection is still the playlist.
  await expect(selected).toHaveCount(1);
  await expect(selected.locator('[data-col="title"]')).toHaveText(selectedTitle);
  await expect(item(page, "Melodic Vox")).toHaveAttribute("aria-selected", "true");
});

test("a playlist deep in folders keeps a name that can be read and opened", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/?deepFolders=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  const label = tree(page).getByText("The Deep End", { exact: true });
  await expect(label).toBeVisible();
  expect((await label.boundingBox())?.width ?? 0).toBeGreaterThan(70);
  await expect(item(page, "The Deep End")).toHaveAttribute("aria-level", "17");
  await label.click();
  await expect(page.getByTestId("browser-title")).toContainText("The Deep End");
});

test("a Library of twenty thousand playlists draws only what is in view, and all of it can be reached", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/?bigTree=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await expect(item(page, "Archive 1")).toBeVisible();
  expect(await tree(page).getByRole("treeitem").count()).toBeLessThan(200);

  await tree(page).evaluate((element) => { element.scrollTop = element.scrollHeight; });
  await expect(item(page, "Archive 20000")).toBeVisible();
  expect(await tree(page).getByRole("treeitem").count()).toBeLessThan(200);
  await item(page, "Archive 20000").click();
  await expect(page.getByTestId("browser-title")).toContainText("Archive 20000");

  await item(page, "Archive 20000").focus();
  await page.keyboard.press("Home");
  await expect(item(page, "All Tracks")).toBeFocused();
  await page.keyboard.press("End");
  await expect(group(page, "Stick TEST").getByRole("treeitem").last()).toBeFocused();
});
