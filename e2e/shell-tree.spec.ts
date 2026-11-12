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
