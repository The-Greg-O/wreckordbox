/**
 * The old tree's actions in the shell's tree (wbx-45.4): the menus of a
 * playlist, a folder, an intelligent playlist, the Playlists heading, a
 * Stick and a Stick's playlist; tracks dropped on a playlist or a Stick's
 * playlist with the duplicate question; a list moved by dragging; Eject; and
 * folders that stay closed across a restart. All with the view shell on,
 * against the mock: its Library and the TEST Stick, whose two libraries hold
 * the same three playlists, so the tree lists them once.
 *
 * Nothing done from the shell's tree may read a Stick through
 * `deviceLibraries` (the path that settles an unfinished export, wbx-45.5):
 * every test that touches TEST checks the mock's count stays at zero.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";

const tree = (page: Page) => page.getByTestId("shell-tree").getByRole("tree", { name: "Tree" });
const playlists = (page: Page) => tree(page).getByRole("group", { name: "Playlists", exact: true });
const library = (page: Page) => tree(page).getByRole("group", { name: "Library", exact: true });
const stick = (page: Page) => tree(page).getByRole("group", { name: "Stick TEST", exact: true });
const inPlaylists = (page: Page, name: string) => playlists(page).getByRole("treeitem", { name, exact: true });
const onStick = (page: Page, name: string) => stick(page).getByRole("treeitem", { name, exact: true });
const status = (page: Page) => page.getByTestId("shell-status");
const rows = (page: Page) => page.getByTestId("shell-working").getByRole("row").filter({ has: page.getByRole("gridcell") });

type Hooks = { __deviceLibrariesReads?: (path: string) => number };
const deviceLibraryReads = (page: Page) =>
  page.evaluate(() => (window as unknown as Hooks).__deviceLibrariesReads?.("/Volumes/TEST") ?? -1);

async function open(page: Page, { protect = false, url = "/?writable=1" } = {}) {
  await page.addInitScript((protectLibrary) => localStorage.setItem("rbl.preferences", JSON.stringify({
    advanced: { protectLibrary },
    view: { viewShell: true },
  })), protect);
  await page.goto(url);
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await expect(tree(page)).toBeVisible();
}

async function menuOn(page: Page, target: Locator, name: string) {
  await target.click({ button: "right" });
  const menu = page.getByRole("menu", { name, exact: true });
  await expect(menu).toBeVisible();
  return menu;
}

/** Drags `from` onto `to` with the drag events themselves, as the old tree's specs do. */
async function drop(page: Page, from: Locator, to: Locator, at?: { clientY: number }) {
  const dataTransfer = await page.evaluateHandle(() => new DataTransfer());
  await from.dispatchEvent("dragstart", { dataTransfer });
  await to.dispatchEvent("dragenter", { dataTransfer, ...at });
  await to.dispatchEvent("dragover", { dataTransfer, ...at });
  await to.dispatchEvent("drop", { dataTransfer, ...at });
  await from.dispatchEvent("dragend", { dataTransfer });
}

test("a playlist's menu creates, renames and deletes, as the old tree's does", async ({ page }) => {
  await open(page);
  let menu = await menuOn(page, inPlaylists(page, "Melodic Vox"), "Playlist");
  await expect(menu.getByRole("menuitem", { name: "Export Playlist" })).toBeVisible();
  await menu.getByRole("menuitem", { name: "Create New Playlist", exact: true }).click();
  await expect(status(page)).toContainText("Created New playlist.");
  const made = inPlaylists(page, "New playlist");
  await expect(made).toBeVisible();

  menu = await menuOn(page, made, "Playlist");
  await menu.getByRole("menuitem", { name: "Rename Playlist" }).click();
  const field = tree(page).getByRole("textbox", { name: "Rename New playlist" });
  await field.fill("Opening Set");
  await field.press("Enter");
  await expect(status(page)).toContainText("Renamed to Opening Set.");
  const renamed = inPlaylists(page, "Opening Set");
  await expect(renamed).toBeVisible();

  // F2 renames too, and Escape puts the name back.
  await renamed.focus();
  await renamed.press("F2");
  const again = tree(page).getByRole("textbox", { name: "Rename Opening Set" });
  await again.fill("Nope");
  await again.press("Escape");
  await expect(inPlaylists(page, "Opening Set")).toBeVisible();

  // The menu key opens the same menu on the focused row.
  await renamed.focus();
  await renamed.press("ContextMenu");
  menu = page.getByRole("menu", { name: "Playlist", exact: true });
  await menu.getByRole("menuitem", { name: "Delete Playlist" }).click();
  await expect(status(page)).toContainText("Deleted Opening Set.");
  await expect(inPlaylists(page, "Opening Set")).toHaveCount(0);
});

test("a folder's, an intelligent playlist's and the Playlists heading's menus are there", async ({ page }) => {
  await open(page);
  let menu = await menuOn(page, inPlaylists(page, "CURRENT"), "Folder");
  await expect(menu.getByRole("menuitem", { name: "Sort Items" })).toBeVisible();
  await menu.getByRole("menuitem", { name: "Create New Folder" }).click();
  await expect(status(page)).toContainText("Created New folder.");
  await expect(inPlaylists(page, "New folder")).toBeVisible();

  menu = await menuOn(page, inPlaylists(page, "Fresh 128s"), "Playlist");
  await menu.getByRole("menuitem", { name: "Edit the Intelligent Playlist" }).click();
  await expect(page.getByRole("dialog", { name: /Intelligent Playlist/ })).toBeVisible();
  await page.keyboard.press("Escape");

  menu = await menuOn(page, page.getByTestId("shell-tree-playlists-heading"), "Playlists");
  await menu.getByRole("menuitem", { name: "Create New Playlist" }).click();
  await expect(status(page)).toContainText("Created New playlist.");
  await expect(inPlaylists(page, "New playlist")).toBeVisible();
});

test("a playlist exports to a Stick from its menu and by being dropped on the Stick", async ({ page }) => {
  await open(page);
  const menu = await menuOn(page, inPlaylists(page, "Melodic Vox"), "Playlist");
  await menu.getByRole("menuitem", { name: "Export Playlist" }).hover();
  await page.getByRole("menuitem", { name: "TEST", exact: true }).click();
  await expect(page.getByText(/^TEST: Updated/)).toContainText("Added 1 playlist.");

  await drop(page, inPlaylists(page, "Hardstyle"), onStick(page, "TEST"));
  await expect(page.getByText(/^TEST: Updated/)).toBeVisible();
  // An export, not a move: the playlist is where it was.
  await expect(inPlaylists(page, "Hardstyle")).toBeVisible();
  expect(await deviceLibraryReads(page)).toBe(0);
});

test("tracks dropped on a playlist are added, and a second drop asks about duplicates", async ({ page }) => {
  await open(page);
  await library(page).getByRole("treeitem", { name: "All Tracks" }).click();
  const target = inPlaylists(page, "Hardstyle");
  await drop(page, rows(page).first(), target);
  await expect(status(page)).toContainText(/Added 1 track to Hardstyle\.|Already in Hardstyle\./);
  await drop(page, rows(page).first(), target);
  const question = page.getByRole("alertdialog", { name: "Duplicate tracks" });
  await question.getByRole("button", { name: "Skip duplicates" }).click();
  await expect(question).toHaveCount(0);
  await expect(status(page)).toContainText("Already in Hardstyle.");
});

test("tracks dropped on a Stick's playlist are added there, with the duplicate question", async ({ page }) => {
  await open(page, { url: "/?writable=1&tracks=4" });
  await inPlaylists(page, "Melodic Vox").click();
  await expect(rows(page).first()).toBeVisible();
  // After the first add the two libraries differ and are listed apart: Device Library first.
  const target = onStick(page, "NP3-TEST-MP3").first();
  await drop(page, rows(page).first(), target);
  await expect(status(page)).toContainText("Added to NP3-TEST-MP3.");
  // The same track again: the Stick's playlist holds it now, so the window asks.
  await drop(page, rows(page).first(), target);
  const question = page.getByRole("alertdialog", { name: "Duplicate tracks" });
  await expect(question).toContainText("NP3-TEST-MP3");
  await question.getByRole("button", { name: "Skip duplicates" }).click();
  await expect(question).toHaveCount(0);
  expect(await deviceLibraryReads(page)).toBe(0);
});

test("a playlist dragged into a folder moves there", async ({ page }) => {
  await open(page);
  const usb = inPlaylists(page, "USB");
  const folder = inPlaylists(page, "CURRENT");
  await expect(usb).toHaveAttribute("aria-level", "1");
  const box = await folder.boundingBox();
  if (!box) throw new Error("no box");
  await drop(page, usb, folder, { clientY: box.y + box.height / 2 });
  await expect(status(page)).toContainText("Moved USB.");
  await expect(inPlaylists(page, "USB")).toHaveAttribute("aria-level", "2");
});

test("a Stick's playlists are made, renamed, deleted and imported from its menus, without settling a write", async ({ page }) => {
  await open(page);
  let menu = await menuOn(page, onStick(page, "TEST"), "Playlists");
  await menu.getByRole("menuitem", { name: "Create New Playlist" }).click();
  await expect(status(page)).toContainText("Created Untitled Playlist.");
  // One library changed, so the tree lists the two apart now.
  await expect(stick(page).getByText("Device Library")).toBeVisible();
  const made = onStick(page, "Untitled Playlist");
  await expect(made).toBeVisible();

  await made.focus();
  await made.press("F2");
  const field = tree(page).getByRole("textbox", { name: "Rename Untitled Playlist" });
  await field.fill("Gig Set");
  await field.press("Enter");
  await expect(status(page)).toContainText("Renamed to Gig Set.");

  await page.evaluate(() => { (window as unknown as { __confirmAnswer?: boolean }).__confirmAnswer = true; });
  menu = await menuOn(page, onStick(page, "Gig Set"), "Playlist");
  await menu.getByRole("menuitem", { name: "Delete Playlist" }).click();
  await expect(status(page)).toContainText("Deleted Gig Set.");
  await expect(onStick(page, "Gig Set")).toHaveCount(0);

  menu = await menuOn(page, onStick(page, "Now Playing Test").first(), "Playlist");
  await menu.getByRole("menuitem", { name: "Import Playlist" }).click();
  await expect(status(page)).toContainText("Imported Now Playing Test.");
  await expect(inPlaylists(page, "Now Playing Test")).toBeVisible();

  expect(await deviceLibraryReads(page)).toBe(0);
});

test("a Stick whose last write did not finish offers only Eject, and takes no drop", async ({ page }) => {
  await open(page);
  type Mount = { __unmountDevice: (p: string) => void; __remountDevice: (p: string, o: unknown) => void };
  // Pulled out and put back with a write left unfinished; the window looks again on focus.
  await page.evaluate(() => {
    (window as unknown as Mount).__unmountDevice("/Volumes/TEST");
    window.dispatchEvent(new Event("focus"));
  });
  await expect(stick(page)).toHaveCount(0);
  await page.evaluate(() => {
    (window as unknown as Mount).__remountDevice("/Volumes/TEST", { pending: true, libraries: [] });
    window.dispatchEvent(new Event("focus"));
  });
  await expect(stick(page)).toContainText("A write to this Stick did not finish");
  const menu = await menuOn(page, onStick(page, "TEST"), "Stick");
  await expect(menu.getByRole("menuitem")).toHaveText(["Eject TEST"]);
  await page.keyboard.press("Escape");
  await drop(page, inPlaylists(page, "Hardstyle"), onStick(page, "TEST"));
  await expect(page.getByText(/^TEST: Updated/)).toHaveCount(0);
  expect(await deviceLibraryReads(page)).toBe(0);
});

test("a Stick ejects from its row and from its menu", async ({ page }) => {
  await open(page);
  await tree(page).getByRole("button", { name: "Eject TEST" }).click();
  await expect(status(page)).toContainText("TEST safely ejected.");
  await expect(stick(page)).toHaveCount(0);

  const other = tree(page).getByRole("group", { name: "Stick DJ STICK", exact: true });
  const menu = await menuOn(page, other.getByRole("treeitem", { name: "DJ STICK" }), "Stick");
  await menu.getByRole("menuitem", { name: "Eject DJ STICK" }).click();
  await expect(status(page)).toContainText("DJ STICK safely ejected.");
  await expect(other).toHaveCount(0);
});

test("a folder closed in the tree stays closed after a restart", async ({ page }) => {
  await open(page);
  await tree(page).getByRole("button", { name: "Collapse CURRENT" }).click();
  await expect(inPlaylists(page, "Melodic Vox")).toHaveCount(0);
  await page.reload();
  await expect(tree(page)).toBeVisible();
  await expect(inPlaylists(page, "CURRENT")).toHaveAttribute("aria-expanded", "false");
  await expect(inPlaylists(page, "Melodic Vox")).toHaveCount(0);
  await tree(page).getByRole("button", { name: "Expand CURRENT" }).click();
  await page.reload();
  await expect(inPlaylists(page, "Melodic Vox")).toBeVisible();
});

test("under Library Protection an edit from the tree is refused and changes nothing", async ({ page }) => {
  await open(page, { protect: true });
  const menu = await menuOn(page, inPlaylists(page, "Melodic Vox"), "Playlist");
  // Greyed, as the old tree's are while the Library is locked.
  await expect(menu.getByRole("menuitem", { name: "Delete Playlist" })).toBeDisabled();
  await page.keyboard.press("Escape");
  // A drop is refused in words.
  await library(page).getByRole("treeitem", { name: "All Tracks" }).click();
  await drop(page, rows(page).first(), inPlaylists(page, "Hardstyle"));
  await expect(status(page)).toContainText("Editing is locked by Library Protection.");
  // And renaming does not start.
  await inPlaylists(page, "Hardstyle").focus();
  await inPlaylists(page, "Hardstyle").press("F2");
  await expect(tree(page).getByRole("textbox")).toHaveCount(0);
});
