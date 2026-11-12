/**
 * The old tree's actions in the shell's tree (wbx-45.4), with the view shell
 * on, against the mock: its Library and the TEST Stick, whose two libraries
 * hold the same three playlists, so the tree lists them once.
 *
 * Every test asserts what the Library or the Stick holds afterwards, in
 * order, through the mock's read-only hooks (`__libraryTree`,
 * `__libraryPlaylistTitles`, `__stickPlaylistTitles`), and every Stick
 * write is checked against the mock's log of Stick writes (`__stickWrites`):
 * a write from the tree starts only after the confirming hold, and Cancel
 * or an early release writes nothing (review findings 5 and 9).
 */
import { expect, test, type Locator, type Page } from "@playwright/test";

const TEST = "/Volumes/TEST";
const tree = (page: Page) => page.getByTestId("shell-tree").getByRole("tree", { name: "Tree" });
const playlists = (page: Page) => tree(page).getByRole("group", { name: "Playlists", exact: true });
const library = (page: Page) => tree(page).getByRole("group", { name: "Library", exact: true });
const stick = (page: Page) => tree(page).getByRole("group", { name: "Stick TEST", exact: true });
const inPlaylists = (page: Page, name: string) => playlists(page).getByRole("treeitem", { name, exact: true });
const onStick = (page: Page, name: string) => stick(page).getByRole("treeitem", { name, exact: true });
const allTracks = (page: Page) => library(page).getByRole("treeitem", { name: "All Tracks" });
const heading = (page: Page) => page.getByTestId("shell-tree-playlists-heading");
const rows = (page: Page) => page.getByTestId("shell-working").getByRole("row").filter({ has: page.getByRole("gridcell") });
const status = (page: Page) => page.getByTestId("shell-status");
const writeDialog = (page: Page) => page.getByRole("alertdialog", { name: /^Write to / });

type Node = { name: string; kind: string; depth: number };
type Hooks = {
  __libraryTree: () => Node[];
  __libraryPlaylistTitles: (name: string) => string[];
  __stickPlaylistTitles: (path: string, format: string, name: string) => string[];
  __stickWrites: () => { method: string; path: string }[];
  __stickRecovered: () => string[];
  __confirmAnswer?: boolean;
  __confirmed?: string[];
  __menu: (id: string) => void;
  __setStickSpare: (path: string, spare: boolean) => void;
  __unmountDevice: (path: string) => void;
  __remountDevice: (path: string) => void;
  __turnLibraryProtection: (on: boolean) => void;
};
const libraryTree = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__libraryTree());
const titlesOf = (page: Page, name: string) => page.evaluate((n) => (window as unknown as Hooks).__libraryPlaylistTitles(n), name);
const stickTitles = (page: Page, name: string, format = "deviceLibrary") =>
  page.evaluate(([n, f]) => (window as unknown as Hooks).__stickPlaylistTitles("/Volumes/TEST", f as string, n as string), [name, format]);
const stickWrites = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__stickWrites());
const recovered = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__stickRecovered());
const answerConfirm = (page: Page, yes: boolean) => page.evaluate((a) => { (window as unknown as Hooks).__confirmAnswer = a; }, yes);
const confirmed = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__confirmed ?? []);
const stickNames = async (page: Page) =>
  (await stick(page).getByRole("treeitem").evaluateAll((els) =>
    els.map((el) => (el.querySelector("[data-name]")?.textContent ?? el.getAttribute("aria-label") ?? "").trim())));
/** The Library's playlists and folders, as "name@depth", top to bottom. */
const shape = async (page: Page) => (await libraryTree(page))
  .filter((n) => n.kind === "playlist" || n.kind === "folder" || n.kind === "smartPlaylist")
  .map((n) => `${n.name}@${n.depth}`);

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

/** Holds the dialog's button down until the write starts. */
async function holdToWrite(page: Page) {
  const dialog = writeDialog(page);
  await dialog.getByRole("button", { name: /Hold to write/ }).hover();
  await page.mouse.down();
  await expect(dialog).toHaveCount(0, { timeout: 5000 });
  await page.mouse.up();
}

/** The first `n` rows of the open list, picked with a click and Shift+click; their titles in order. */
async function pick(page: Page, n: number) {
  await rows(page).first().locator('[data-col="title"]').click();
  if (n > 1) await rows(page).nth(n - 1).locator('[data-col="title"]').click({ modifiers: ["Shift"] });
  return (await rows(page).locator('[data-col="title"]').allTextContents()).slice(0, n).map((t) => t.trim());
}

test.describe("Library playlists and folders", () => {
  test("a playlist's menu makes one beside it, renames it, and deletes it after asking", async ({ page }) => {
    await open(page);
    const before = await shape(page);
    const at = before.indexOf("Melodic Vox@2");
    let menu = await menuOn(page, inPlaylists(page, "Melodic Vox"), "Playlist");
    await menu.getByRole("menuitem", { name: "Create New Playlist", exact: true }).click();
    await expect(inPlaylists(page, "New playlist")).toBeVisible();
    // Directly below the playlist the menu was opened on, at its depth, empty.
    expect(await shape(page)).toEqual([...before.slice(0, at + 1), "New playlist@2", ...before.slice(at + 1)]);
    expect(await titlesOf(page, "New playlist")).toEqual([]);

    menu = await menuOn(page, inPlaylists(page, "New playlist"), "Playlist");
    await menu.getByRole("menuitem", { name: "Rename Playlist" }).click();
    const field = tree(page).getByRole("textbox", { name: "Rename New playlist" });
    await field.fill("Opening Set");
    await field.press("Enter");
    await expect(inPlaylists(page, "Opening Set")).toBeVisible();
    // The keyboard is back on the renamed row.
    await expect(inPlaylists(page, "Opening Set")).toBeFocused();
    expect(await shape(page)).toEqual([...before.slice(0, at + 1), "Opening Set@2", ...before.slice(at + 1)]);

    // F2, then Escape, puts the name back and changes nothing.
    await inPlaylists(page, "Opening Set").press("F2");
    await tree(page).getByRole("textbox", { name: "Rename Opening Set" }).fill("Nope");
    await tree(page).getByRole("textbox", { name: "Rename Opening Set" }).press("Escape");
    await expect(inPlaylists(page, "Opening Set")).toBeFocused();
    expect(await shape(page)).toContain("Opening Set@2");

    // Delete asks first, naming it; Cancel keeps it.
    await answerConfirm(page, false);
    menu = await menuOn(page, inPlaylists(page, "Opening Set"), "Playlist");
    await menu.getByRole("menuitem", { name: "Delete Playlist" }).click();
    await expect.poll(() => confirmed(page)).toContain(
      "Delete the playlist “Opening Set”? It lists 0 tracks. The tracks stay in the Library, and Undo brings the playlist back.",
    );
    expect(await shape(page)).toContain("Opening Set@2");
    await answerConfirm(page, true);
    menu = await menuOn(page, inPlaylists(page, "Opening Set"), "Playlist");
    await menu.getByRole("menuitem", { name: "Delete Playlist" }).click();
    await expect(inPlaylists(page, "Opening Set")).toHaveCount(0);
    expect(await shape(page)).toEqual(before);
  });

  test("a populated folder is deleted only after a question naming what is in it, and Undo restores it all", async ({ page }) => {
    await open(page);
    const before = await shape(page);
    const vox = await titlesOf(page, "Melodic Vox");
    await answerConfirm(page, false);
    let menu = await menuOn(page, inPlaylists(page, "CURRENT"), "Folder");
    await menu.getByRole("menuitem", { name: "Delete Folder" }).click();
    await expect.poll(async () => (await confirmed(page)).at(-1) ?? "").toMatch(
      /^Delete the folder “CURRENT” and the \d+ playlists?( and \d+ folders?)? in it\? They list \d+ tracks?\. The tracks stay in the Library, and Undo brings the folder back\.$/,
    );
    expect(await shape(page)).toEqual(before);

    await answerConfirm(page, true);
    menu = await menuOn(page, inPlaylists(page, "CURRENT"), "Folder");
    await menu.getByRole("menuitem", { name: "Delete Folder" }).click();
    await expect(inPlaylists(page, "CURRENT")).toHaveCount(0);
    await expect(inPlaylists(page, "Melodic Vox")).toHaveCount(0);

    await page.evaluate(() => { (window as unknown as Hooks).__menu("undo"); });
    await expect(inPlaylists(page, "CURRENT")).toBeVisible();
    expect(await shape(page)).toEqual(before);
    expect(await titlesOf(page, "Melodic Vox")).toEqual(vox);
  });

  test("Sort Items puts a folder's lists in name order", async ({ page }) => {
    await open(page);
    const children = async () => {
      const all = await libraryTree(page);
      const start = all.findIndex((n) => n.name === "CURRENT");
      const depth = all[start]?.depth ?? 0;
      const out: string[] = [];
      for (const n of all.slice(start + 1)) {
        if (n.depth <= depth) break;
        if (n.depth === depth + 1) out.push(n.name);
      }
      return out;
    };
    const before = await children();
    const sorted = [...before].sort((a, b) => a.localeCompare(b, "en", { sensitivity: "base" }));
    expect(before).not.toEqual(sorted);
    const menu = await menuOn(page, inPlaylists(page, "CURRENT"), "Folder");
    await menu.getByRole("menuitem", { name: "Sort Items" }).click();
    await expect.poll(children).toEqual(sorted);
  });

  test("an intelligent playlist is edited to the end from its menu", async ({ page }) => {
    await open(page);
    const menu = await menuOn(page, inPlaylists(page, "Fresh 128s"), "Playlist");
    await menu.getByRole("menuitem", { name: "Edit the Intelligent Playlist" }).click();
    const editor = page.getByRole("dialog", { name: "Edit the Intelligent Playlist" });
    await editor.getByLabel("List name").fill("Fresh 130s");
    const condition = editor.getByRole("group", { name: "Condition 1" });
    await condition.getByLabel("Property", { exact: true }).selectOption("genre");
    await condition.getByLabel("Value", { exact: true }).first().fill("House");
    await editor.getByRole("button", { name: "OK" }).click();
    await expect(inPlaylists(page, "Fresh 130s")).toBeVisible();
    await expect(inPlaylists(page, "Fresh 128s")).toHaveCount(0);
    expect((await shape(page)).filter((n) => n.startsWith("Fresh 1"))).toEqual(["Fresh 130s@2"]);
  });

  test("a playlist dragged into a folder lands at the folder's end", async ({ page }) => {
    await open(page);
    const before = await shape(page);
    const box = await inPlaylists(page, "CURRENT").boundingBox();
    if (!box) throw new Error("no box");
    await drop(page, inPlaylists(page, "USB"), inPlaylists(page, "CURRENT"), { clientY: box.y + box.height / 2 });
    await expect(inPlaylists(page, "USB")).toHaveAttribute("aria-level", "2");
    const after = await shape(page);
    // Moved, not copied: one USB, now the last of CURRENT's children, its own contents still under it.
    expect(after.filter((n) => n.startsWith("USB@"))).toEqual(["USB@2"]);
    const current = after.indexOf("CURRENT@1");
    const end = after.findIndex((n, i) => i > current && n.endsWith("@1"));
    const children = after.slice(current + 1, end < 0 ? after.length : end).filter((n) => n.endsWith("@2"));
    expect(children.at(-1)).toBe("USB@2");
    expect(after.length).toBe(before.length);
  });
});

test.describe("tracks dropped on a Library playlist", () => {
  test("several tracks go on in order; a second drop asks, and Skip, Cancel and Add do what they say", async ({ page }) => {
    await open(page);
    await allTracks(page).click();
    const picked = await pick(page, 3);
    const before = await titlesOf(page, "Hardstyle");
    const fresh = picked.filter((t) => !before.includes(t));
    const target = inPlaylists(page, "Hardstyle");
    await drop(page, rows(page).first(), target);
    await expect.poll(() => titlesOf(page, "Hardstyle")).toEqual([...before, ...fresh]);

    const question = page.getByRole("alertdialog", { name: "Duplicate tracks" });
    await drop(page, rows(page).first(), target);
    await question.getByRole("button", { name: "Skip duplicates" }).click();
    await expect(question).toHaveCount(0);
    expect(await titlesOf(page, "Hardstyle")).toEqual([...before, ...fresh]);

    await drop(page, rows(page).first(), target);
    await question.getByRole("button", { name: "Cancel" }).click();
    await expect(question).toHaveCount(0);
    expect(await titlesOf(page, "Hardstyle")).toEqual([...before, ...fresh]);

    await drop(page, rows(page).first(), target);
    await question.getByRole("button", { name: "Add duplicates" }).click();
    await expect.poll(() => titlesOf(page, "Hardstyle")).toEqual([...before, ...fresh, ...picked]);
  });

  test("a folder, an intelligent playlist and All Tracks take no tracks", async ({ page }) => {
    await open(page);
    await allTracks(page).click();
    await pick(page, 1);
    const before = await shape(page);
    const fresh = await titlesOf(page, "Fresh 128s");
    for (const target of [inPlaylists(page, "CURRENT"), inPlaylists(page, "Fresh 128s"), allTracks(page)]) {
      await drop(page, rows(page).first(), target);
    }
    await page.waitForTimeout(300);
    expect(await shape(page)).toEqual(before);
    expect(await titlesOf(page, "Fresh 128s")).toEqual(fresh);
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
  });
});

test.describe("files from the desktop", () => {
  test("the native drop finds the shell's playlists, folders, All Tracks and the Playlists heading", async ({ page }) => {
    await open(page);
    // The window's native drop (Linux) asks which element is under the point; this is that question.
    const under = (target: Locator) => target.evaluate((el) => {
      const box = el.getBoundingClientRect();
      const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
      return hit?.closest<HTMLElement>("[data-file-drop-playlist]")?.dataset.fileDropPlaylist ?? null;
    });
    expect(await under(allTracks(page))).toBe("all");
    expect(await under(heading(page))).toBe("playlists");
    expect(await under(inPlaylists(page, "Hardstyle"))).toBe(await inPlaylists(page, "Hardstyle").getAttribute("data-row-id"));
    expect(await under(inPlaylists(page, "CURRENT"))).toBe(await inPlaylists(page, "CURRENT").getAttribute("data-row-id"));
    // Not a Stick's playlist, not a queue.
    expect(await under(onStick(page, "Melodic Vox"))).toBeNull();
    expect(await under(library(page).getByRole("treeitem", { name: "Intake" }))).toBeNull();
  });

  test("a file dropped on a shell playlist is imported into that playlist", async ({ page }) => {
    await open(page, { url: "/?writable=1&dropFixture=1" });
    const before = await titlesOf(page, "Hardstyle");
    const transfer = await page.evaluateHandle(() => {
      const data = new DataTransfer();
      data.items.add(new File(["fixture audio"], "Shell Drop 45.mp3", { type: "audio/mpeg" }));
      Object.defineProperty(data.files[0], "path", { value: "/tmp/Shell Drop 45.mp3" });
      return data;
    });
    const target = inPlaylists(page, "Hardstyle");
    await target.dispatchEvent("dragover", { dataTransfer: transfer });
    await target.dispatchEvent("drop", { dataTransfer: transfer });
    await expect.poll(() => titlesOf(page, "Hardstyle")).toEqual([...before, "Shell Drop 45"]);
  });
});

test.describe("from the keyboard", () => {
  test("the menu key opens a menu that has the keyboard; arrows, Enter and Escape work it, and the row gets the keyboard back", async ({ page }) => {
    await open(page);
    const vox = inPlaylists(page, "Melodic Vox");
    await vox.click();
    await vox.press("ContextMenu");
    const menu = page.getByRole("menu", { name: "Playlist", exact: true });
    await expect(menu.getByRole("menuitem").first()).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
    await expect(vox).toBeFocused();

    // Down to Create New Folder, Enter: made; the keyboard is back on the row.
    const before = await shape(page);
    await vox.press("ContextMenu");
    const live = await menu.getByRole("menuitem").evaluateAll((els) =>
      els.filter((el) => !(el as HTMLButtonElement).disabled).map((el) => el.textContent?.trim() ?? ""));
    for (let i = 0; i < live.indexOf("Create New Folder"); i += 1) await page.keyboard.press("ArrowDown");
    await expect(menu.getByRole("menuitem", { name: "Create New Folder" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(inPlaylists(page, "New folder")).toBeVisible();
    expect((await shape(page)).length).toBe(before.length + 1);
    await expect(vox).toBeFocused();
  });

  test("Up from the first list reaches the Playlists heading, whose menu opens on Enter", async ({ page }) => {
    await open(page);
    const first = inPlaylists(page, "CURRENT");
    await first.click();
    await first.press("ArrowUp");
    await expect(heading(page)).toBeFocused();
    await page.keyboard.press("Enter");
    const menu = page.getByRole("menu", { name: "Playlists", exact: true });
    await expect(menu.getByRole("menuitem", { name: "Create New Playlist" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(inPlaylists(page, "New playlist")).toBeVisible();
    expect(await shape(page)).toContain("New playlist@1");
    await expect(heading(page)).toBeFocused();
  });

  test("Create New Playlist from the palette works with All Tracks chosen in the tree", async ({ page }) => {
    await open(page);
    await allTracks(page).click();
    await page.keyboard.press("Control+Shift+P");
    const palette = page.getByRole("dialog", { name: "Command palette" });
    await palette.getByRole("combobox", { name: "Search actions" }).fill("Create New Playlist");
    const entry = palette.getByRole("listbox", { name: "Actions" }).getByRole("option").first();
    await expect(entry).toHaveAccessibleName(/Create New Playlist/);
    await expect(entry).not.toHaveAttribute("aria-disabled", "true");
    await page.keyboard.press("Enter");
    await expect(inPlaylists(page, "New playlist")).toBeVisible();
    // At the top of the playlists, as the old tree makes one from All Tracks.
    expect((await shape(page))[0]).toBe("New playlist@1");
  });
});

test.describe("writes to a Stick", () => {
  test("tracks dropped on a Stick playlist are written only after the hold; Cancel and an early release write nothing", async ({ page }) => {
    await open(page, { url: "/?writable=1&tracks=4" });
    await inPlaylists(page, "Melodic Vox").click();
    await expect(rows(page).first()).toBeVisible();
    const before = await stickTitles(page, "NP3-TEST-MP3");
    const target = onStick(page, "NP3-TEST-MP3");

    await drop(page, rows(page).first(), target);
    const dialog = writeDialog(page);
    await expect(dialog).toContainText("TEST · Gig stick");
    await expect(dialog).toContainText("Device Library: 1 track added at the end of “NP3-TEST-MP3”");
    await expect(dialog).toContainText("Hold the button for 2 seconds");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toHaveCount(0);
    expect(await stickWrites(page)).toEqual([]);
    expect(await stickTitles(page, "NP3-TEST-MP3")).toEqual(before);

    // Let go after half a second: nothing.
    await drop(page, rows(page).first(), target);
    await dialog.getByRole("button", { name: /Hold to write/ }).hover();
    await page.mouse.down();
    await page.waitForTimeout(500);
    await page.mouse.up();
    await page.waitForTimeout(2000);
    await expect(dialog).toBeVisible();
    expect(await stickWrites(page)).toEqual([]);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);

    await drop(page, rows(page).first(), target);
    await holdToWrite(page);
    await expect.poll(async () => (await stickTitles(page, "NP3-TEST-MP3")).length).toBe(before.length + 1);
    expect((await stickWrites(page)).every((w) => w.path === TEST)).toBe(true);
    expect(await recovered(page)).toEqual([]);
  });

  test("a Spare stick asks for the shorter hold", async ({ page }) => {
    await open(page);
    await page.evaluate((path) => {
      const h = window as unknown as Hooks;
      h.__setStickSpare(path, true);
      h.__unmountDevice(path);
      window.dispatchEvent(new Event("focus"));
    }, TEST);
    await expect(stick(page)).toHaveCount(0);
    await page.evaluate((path) => {
      (window as unknown as Hooks).__remountDevice(path);
      window.dispatchEvent(new Event("focus"));
    }, TEST);
    await expect(onStick(page, "Melodic Vox")).toBeVisible();
    const menu = await menuOn(page, onStick(page, "TEST"), "Playlists");
    await menu.getByRole("menuitem", { name: "Create New Playlist" }).click();
    const dialog = writeDialog(page);
    await expect(dialog).toContainText("TEST · Spare stick");
    await expect(dialog).toContainText("Hold the button for 1 second");
    await page.keyboard.press("Escape");
    expect(await stickWrites(page)).toEqual([]);
  });

  test("Export Playlist shows the export's own plan, writes nothing on Cancel, and exports on the hold", async ({ page }) => {
    await open(page);
    const menu = await menuOn(page, inPlaylists(page, "Hardstyle"), "Playlist");
    await menu.getByRole("menuitem", { name: "Export Playlist" }).hover();
    await page.getByRole("menuitem", { name: "TEST", exact: true }).click();
    const dialog = writeDialog(page);
    await expect(dialog).toContainText("Export the playlist “Hardstyle” to TEST");
    await expect(dialog).toContainText("Playlist “Hardstyle” created");
    await expect(dialog).toContainText(/\d+ tracks? copied to the Stick/);
    await dialog.getByRole("button", { name: "Cancel" }).click();
    expect(await stickWrites(page)).toEqual([]);

    // Dropped on the Stick: the same plan, and the playlist stays where it was.
    const before = await shape(page);
    await drop(page, inPlaylists(page, "Hardstyle"), onStick(page, "TEST"));
    await expect(dialog).toContainText("Export the playlist “Hardstyle” to TEST");
    await holdToWrite(page);
    await expect.poll(() => stickWrites(page)).toEqual([{ method: "exportPlaylist", path: TEST }]);
    expect(await shape(page)).toEqual(before);
  });

  test("tracks dropped on the Stick itself are exported after the hold, with the playlists it holds named", async ({ page }) => {
    await open(page);
    // A first export, so the Stick holds a playlist of this Library.
    const menu = await menuOn(page, inPlaylists(page, "Hardstyle"), "Playlist");
    await menu.getByRole("menuitem", { name: "Export Playlist" }).hover();
    await page.getByRole("menuitem", { name: "TEST", exact: true }).click();
    await holdToWrite(page);
    await expect.poll(async () => (await stickWrites(page)).length).toBe(1);
    await expect(onStick(page, "TEST")).toBeVisible();

    await allTracks(page).click();
    await pick(page, 2);
    await expect.poll(async () => {
      await drop(page, rows(page).first(), onStick(page, "TEST"));
      return writeDialog(page).count();
    }).toBe(1);
    const dialog = writeDialog(page);
    await expect(dialog).toContainText("Export 2 tracks to TEST");
    // Export Track writes again every playlist the Stick holds from this Library.
    await expect(dialog).toContainText("Playlist “Hardstyle” written again from the Library");
    await dialog.getByRole("button", { name: "Cancel" }).click();
    expect((await stickWrites(page)).length).toBe(1);
    await drop(page, rows(page).first(), onStick(page, "TEST"));
    await holdToWrite(page);
    await expect.poll(() => stickWrites(page)).toEqual([
      { method: "exportPlaylist", path: TEST }, { method: "exportTracksToDevice", path: TEST },
    ]);
  });

  test("a Stick's playlists are made, renamed, deleted and imported, each write held for", async ({ page }) => {
    await open(page);
    let menu = await menuOn(page, onStick(page, "TEST"), "Playlists");
    await menu.getByRole("menuitem", { name: "Create New Playlist" }).click();
    await expect(writeDialog(page)).toContainText("Playlist “Untitled Playlist” created in Device Library");
    await holdToWrite(page);
    // One library changed, so the two are listed apart: Untitled Playlist in Device Library alone.
    await expect(stick(page).getByRole("treeitem", { name: "Device Library" })).toBeVisible();
    expect(await stickNames(page)).toEqual([
      "TEST", "Device Library", "Untitled Playlist", "NP3-TEST-MP3", "Melodic Vox", "Now Playing Test",
      "OneLibrary", "NP3-TEST-MP3", "Melodic Vox", "Now Playing Test",
    ]);

    await onStick(page, "Untitled Playlist").click();
    await onStick(page, "Untitled Playlist").press("F2");
    await tree(page).getByRole("textbox", { name: "Rename Untitled Playlist" }).fill("Gig Set");
    await tree(page).getByRole("textbox", { name: "Rename Untitled Playlist" }).press("Enter");
    await expect(writeDialog(page)).toContainText("Device Library: “Untitled Playlist” renamed “Gig Set”");
    await holdToWrite(page);
    await expect(onStick(page, "Gig Set")).toBeVisible();

    menu = await menuOn(page, onStick(page, "Gig Set"), "Playlist");
    await menu.getByRole("menuitem", { name: "Delete Playlist" }).click();
    await expect(writeDialog(page)).toContainText("Playlist “Gig Set” (0 tracks) deleted from Device Library");
    await holdToWrite(page);
    await expect(onStick(page, "Gig Set")).toHaveCount(0);
    // rekordbox's own OK/Cancel is not asked again on top of the hold.
    expect(await confirmed(page)).toEqual([]);

    const fromStick = await stickTitles(page, "Now Playing Test");
    const libraryBefore = await shape(page);
    menu = await menuOn(page, onStick(page, "Now Playing Test").first(), "Playlist");
    await menu.getByRole("menuitem", { name: "Import Playlist" }).click();
    await expect(inPlaylists(page, "Now Playing Test")).toBeVisible();
    // A new playlist at the end of the top level, one entry per Stick entry (each the collection's own track).
    expect(await shape(page)).toEqual([...libraryBefore, "Now Playing Test@1"]);
    expect((await titlesOf(page, "Now Playing Test")).length).toBe(fromStick.length);
    expect(await recovered(page)).toEqual([]);
  });

  test("on a Stick whose two libraries differ, each library's label makes a playlist in that library", async ({ page }) => {
    await open(page);
    let menu = await menuOn(page, onStick(page, "TEST"), "Playlists");
    await menu.getByRole("menuitem", { name: "Create New Playlist" }).click();
    await holdToWrite(page);
    const one = stick(page).getByRole("treeitem", { name: "OneLibrary" });
    await expect(one).toBeVisible();
    // Reached and worked from the keyboard.
    await one.focus();
    await one.press("Enter");
    menu = page.getByRole("menu", { name: "Playlists", exact: true });
    await expect(menu.getByRole("menuitem", { name: "Create New Playlist" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(writeDialog(page)).toContainText("Playlist “Untitled Playlist” created in OneLibrary");
    await holdToWrite(page);
    // Both libraries hold it now, so they list the same again and the tree shows one list.
    await expect(stick(page).getByRole("treeitem", { name: "Device Library" })).toHaveCount(0);
    expect(await stickNames(page)).toEqual(["TEST", "Untitled Playlist", "NP3-TEST-MP3", "Melodic Vox", "Now Playing Test"]);
  });

  test("a Stick ejects from its row and from its menu, with no hold", async ({ page }) => {
    await open(page);
    await tree(page).getByRole("button", { name: "Eject TEST" }).click();
    await expect(status(page)).toContainText("TEST safely ejected.");
    await expect(stick(page)).toHaveCount(0);

    const other = tree(page).getByRole("group", { name: "Stick DJ STICK", exact: true });
    const menu = await menuOn(page, other.getByRole("treeitem", { name: "DJ STICK" }), "Stick");
    await menu.getByRole("menuitem", { name: "Eject DJ STICK" }).click();
    await expect(other).toHaveCount(0);
    expect(await stickWrites(page)).toEqual([]);
  });
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

test("under Library Protection the tree's edits are refused and the Library is unchanged", async ({ page }) => {
  await open(page, { protect: true });
  const before = await shape(page);
  const hardstyle = await titlesOf(page, "Hardstyle");
  const menu = await menuOn(page, inPlaylists(page, "Melodic Vox"), "Playlist");
  for (const name of ["Create New Playlist", "Delete Playlist", "Rename Playlist"]) {
    await expect(menu.getByRole("menuitem", { name, exact: true })).toBeDisabled();
  }
  await page.keyboard.press("Escape");
  await allTracks(page).click();
  await pick(page, 1);
  await drop(page, rows(page).first(), inPlaylists(page, "Hardstyle"));
  await expect(status(page)).toContainText("Editing is locked by Library Protection.");
  await inPlaylists(page, "Hardstyle").press("F2");
  await expect(tree(page).getByRole("textbox")).toHaveCount(0);
  // No file target either.
  await expect(inPlaylists(page, "Hardstyle")).not.toHaveAttribute("data-file-drop-playlist", /.*/);
  expect(await shape(page)).toEqual(before);
  expect(await titlesOf(page, "Hardstyle")).toEqual(hardstyle);
});

test("Library Protection turned on while a menu is open greys its edits there and then", async ({ page }) => {
  await open(page);
  const menu = await menuOn(page, inPlaylists(page, "Melodic Vox"), "Playlist");
  await expect(menu.getByRole("menuitem", { name: "Delete Playlist" })).toBeEnabled();
  await page.evaluate(() => { (window as unknown as Hooks).__turnLibraryProtection(true); });
  await expect(menu.getByRole("menuitem", { name: "Delete Playlist" })).toBeDisabled();
});
