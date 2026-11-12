/**
 * Every write to a Stick, from every part of the window, goes through one
 * staged dialog and an approval token (wbx-45.4, second review).
 *
 * The mock enforces the backend's rule: a Stick-writing call without a
 * matching, unused token from the plan is refused (`__stickRefusedWrites`)
 * and writes nothing; `__stickWrites` holds what ran. These tests cover the
 * hold (focus loss, key repeat, a request replaced while held, focus kept
 * in the dialog), the token's binding (a Stick changed or replaced after
 * the plan), and the surfaces outside the tree: the track list's Export
 * Track, a Stick playlist's Remove, the device panel's settings and the
 * Sync Manager, in the old layout as well as the shell.
 */
import { expect, test, type Page } from "@playwright/test";

const TEST = "/Volumes/TEST";
type Hooks = {
  __stickWrites: () => { method: string; path: string; args?: unknown }[];
  __stickRefusedWrites: () => { method: string; path: string }[];
  __stickPlaylistTitles: (path: string, format: string, name: string) => string[];
  __touchStick: (path: string) => void;
  __replaceStick: (path: string) => void;
};
const writes = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__stickWrites().map((w) => w.method));
const refused = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__stickRefusedWrites().map((w) => w.method));
const stickTitles = (page: Page, name: string) =>
  page.evaluate((n) => (window as unknown as Hooks).__stickPlaylistTitles("/Volumes/TEST", "deviceLibrary", n), name);
const writeDialog = (page: Page) => page.getByRole("alertdialog", { name: /^Write to / });
const holdButton = (page: Page) => writeDialog(page).getByRole("button", { name: /Hold to write/ });
const tree = (page: Page) => page.getByTestId("shell-tree").getByRole("tree", { name: "Tree" });
const stick = (page: Page) => tree(page).getByRole("group", { name: "Stick TEST", exact: true });
const inPlaylists = (page: Page, name: string) =>
  tree(page).getByRole("group", { name: "Playlists", exact: true }).getByRole("treeitem", { name, exact: true });
const rows = (page: Page) => page.getByRole("row").filter({ has: page.getByRole("gridcell") });

async function open(page: Page, shell = true) {
  await page.addInitScript((viewShell) => localStorage.setItem("rbl.preferences", JSON.stringify({
    advanced: { protectLibrary: false }, view: { viewShell },
  })), shell);
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
}

/** Create New Playlist on TEST from its row's menu in the shell's tree: a staged write. */
async function stageCreate(page: Page) {
  await stick(page).getByRole("treeitem", { name: "TEST", exact: true }).click({ button: "right" });
  await page.getByRole("menu", { name: "Playlists" }).getByRole("menuitem", { name: "Create New Playlist" }).click();
  await expect(writeDialog(page)).toBeVisible();
}

async function hold(page: Page, ms: number) {
  await holdButton(page).hover();
  await page.mouse.down();
  await page.waitForTimeout(ms);
}

test.describe("the hold", () => {
  test("the window losing focus ends a hold, and only a fresh press starts it again", async ({ page }) => {
    await open(page);
    await stageCreate(page);
    await hold(page, 500);
    await page.evaluate(() => window.dispatchEvent(new Event("blur")));
    await page.waitForTimeout(2200);
    await expect(writeDialog(page)).toBeVisible();
    await expect(writeDialog(page)).toContainText("The hold was interrupted. Press and hold again to write.");
    expect(await writes(page)).toEqual([]);
    await page.mouse.up();
    // A fresh press, held to the end: written.
    await hold(page, 0);
    await expect(writeDialog(page)).toHaveCount(0, { timeout: 4000 });
    await page.mouse.up();
    await expect.poll(() => writes(page)).toEqual(["devicePlaylistEdit"]);
  });

  test("a key held: repeats start nothing, letting go early writes nothing, a full hold writes", async ({ page }) => {
    await open(page);
    await stageCreate(page);
    // Repeats alone, as a key already down when the dialog opened sends.
    for (let i = 0; i < 25; i += 1) await holdButton(page).dispatchEvent("keydown", { key: "Enter", repeat: true });
    await page.waitForTimeout(2200);
    expect(await writes(page)).toEqual([]);
    await holdButton(page).focus();
    await page.keyboard.down("Enter");
    await page.waitForTimeout(600);
    await page.keyboard.up("Enter");
    await page.waitForTimeout(2000);
    await expect(writeDialog(page)).toBeVisible();
    expect(await writes(page)).toEqual([]);
    await page.keyboard.down("Enter");
    await expect(writeDialog(page)).toHaveCount(0, { timeout: 4000 });
    await page.keyboard.up("Enter");
    await expect.poll(() => writes(page)).toEqual(["devicePlaylistEdit"]);
  });

  test("a second request replaces the first while it is held: the first is never written", async ({ page }) => {
    await open(page);
    await stageCreate(page);
    await hold(page, 500);
    // A playlist dropped on the Stick behind the dialog: another write asked for.
    const dataTransfer = await page.evaluateHandle(() => new DataTransfer());
    await inPlaylists(page, "Hardstyle").dispatchEvent("dragstart", { dataTransfer });
    for (const type of ["dragenter", "dragover", "drop"]) {
      await stick(page).getByRole("treeitem", { name: "TEST", exact: true }).dispatchEvent(type, { dataTransfer });
    }
    await expect(writeDialog(page)).toContainText("Export the playlist “Hardstyle” to TEST");
    // Still holding: the first request's time is up, and it was cancelled.
    await page.waitForTimeout(2200);
    expect(await writes(page)).toEqual([]);
    await page.mouse.up();
    await writeDialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(writeDialog(page)).toHaveCount(0);
    expect(await writes(page)).toEqual([]);
  });

  test("the dialog keeps the focus: Tab goes round its two buttons", async ({ page }) => {
    await open(page);
    await stageCreate(page);
    const cancel = writeDialog(page).getByRole("button", { name: "Cancel" });
    await expect(cancel).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(holdButton(page)).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(cancel).toBeFocused();
    await page.keyboard.press("Shift+Tab");
    await expect(holdButton(page)).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(writeDialog(page)).toHaveCount(0);
    expect(await writes(page)).toEqual([]);
  });
});

test.describe("the approval is for that Stick as it was planned", () => {
  test("a Stick changed after the plan refuses the write and asks for a fresh plan", async ({ page }) => {
    await open(page);
    await stageCreate(page);
    await page.evaluate((path) => { (window as unknown as Hooks).__touchStick(path); }, TEST);
    await hold(page, 0);
    await expect(writeDialog(page)).toHaveCount(0, { timeout: 4000 });
    await page.mouse.up();
    await expect(page.getByTestId("shell-status")).toContainText("The Stick changed since the write was confirmed");
    expect(await writes(page)).toEqual([]);
    expect(await refused(page)).toEqual(["devicePlaylistEdit"]);
  });

  test("another Stick at the same place refuses the write", async ({ page }) => {
    await open(page);
    await stageCreate(page);
    await page.evaluate((path) => { (window as unknown as Hooks).__replaceStick(path); }, TEST);
    await hold(page, 0);
    await expect(writeDialog(page)).toHaveCount(0, { timeout: 4000 });
    await page.mouse.up();
    await expect(page.getByTestId("shell-status")).toContainText("not the one the write was confirmed for");
    expect(await writes(page)).toEqual([]);
  });
});

test.describe("every surface that writes to a Stick", () => {
  test("the track list's Export Track is staged; Cancel writes nothing, the hold exports exactly those tracks", async ({ page }) => {
    await open(page);
    await tree(page).getByRole("group", { name: "Library", exact: true }).getByRole("treeitem", { name: "All Tracks" }).click();
    const title = (await rows(page).first().locator('[data-col="title"]').innerText()).trim();
    const exportTo = async () => {
      await rows(page).first().locator('[data-col="title"]').click({ button: "right" });
      await page.getByRole("menu", { name: "Track" }).getByRole("menuitem", { name: "Export Track" }).hover();
      await page.getByRole("menuitem", { name: "TEST", exact: true }).click();
      await expect(writeDialog(page)).toContainText("Export 1 track to TEST");
    };
    await exportTo();
    await writeDialog(page).getByRole("button", { name: "Cancel" }).click();
    expect(await writes(page)).toEqual([]);
    await exportTo();
    await hold(page, 0);
    await expect(writeDialog(page)).toHaveCount(0, { timeout: 4000 });
    await page.mouse.up();
    await expect.poll(() => writes(page)).toEqual(["exportTracksToDevice"]);
    const holds = await page.evaluate(() =>
      (window as unknown as { __stickHolds: (p: string) => { loose: string[] } }).__stickHolds("/Volumes/TEST"));
    expect(holds.loose).toEqual([title]);
  });

  test("a Stick playlist's Remove from Playlist is staged; Cancel keeps the track, the hold removes exactly it", async ({ page }) => {
    await open(page);
    await stick(page).getByRole("treeitem", { name: "Melodic Vox" }).click();
    await expect(rows(page)).toHaveCount(3);
    const before = await stickTitles(page, "Melodic Vox");
    const remove = async () => {
      await rows(page).first().click();
      await rows(page).first().click({ button: "right" });
      await page.getByRole("menu", { name: "Track" }).getByRole("menuitem", { name: "Remove from Playlist" }).click();
      await expect(writeDialog(page)).toContainText("Device Library: 1 entry removed from “Melodic Vox”");
    };
    await remove();
    await writeDialog(page).getByRole("button", { name: "Cancel" }).click();
    expect(await stickTitles(page, "Melodic Vox")).toEqual(before);
    await remove();
    await hold(page, 0);
    await expect(writeDialog(page)).toHaveCount(0, { timeout: 4000 });
    await page.mouse.up();
    await expect.poll(() => stickTitles(page, "Melodic Vox")).toEqual(before.slice(1));
  });

  test("opening a Stick's panel writes nothing, and a settings change is staged", async ({ page }) => {
    await open(page, false);
    await page.getByRole("tablist", { name: "Library sources" }).getByRole("tab", { name: "Devices" }).click();
    await page.getByRole("treeitem", { name: /TEST/ }).first().click();
    const panel = page.getByRole("region", { name: "Device TEST" });
    const colour = panel.getByRole("combobox", { name: "Background Color : OneLibrary" });
    await expect(colour).toHaveValue("0");
    expect(await writes(page)).toEqual([]);
    await colour.selectOption({ label: "Purple" });
    await expect(writeDialog(page)).toContainText("Change the settings on TEST");
    await writeDialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(colour).toHaveValue("0");
    expect(await writes(page)).toEqual([]);
    await colour.selectOption({ label: "Purple" });
    await hold(page, 0);
    await expect(writeDialog(page)).toHaveCount(0, { timeout: 4000 });
    await page.mouse.up();
    await expect.poll(() => writes(page)).toEqual(["saveDeviceSettings"]);
    await expect(colour).toHaveValue("8");
  });

  test("the old layout's Devices tree stages its writes too", async ({ page }) => {
    await open(page, false);
    await page.getByRole("tablist", { name: "Library sources" }).getByRole("tab", { name: "Devices" }).click();
    await page.getByRole("button", { name: "Expand TEST" }).dispatchEvent("mousedown");
    await page.locator('[role="treeitem"][data-kind="devicePlaylists"]').first().click({ button: "right" });
    await page.getByRole("menu", { name: "Playlists" }).getByRole("menuitem", { name: "Create New Playlist" }).click();
    await expect(writeDialog(page)).toContainText("Playlist “Untitled Playlist” created in Device Library");
    await writeDialog(page).getByRole("button", { name: "Cancel" }).click();
    expect(await writes(page)).toEqual([]);
    await expect(page.locator('[role="treeitem"][data-kind="devicePlaylist"]').filter({ hasText: "Untitled Playlist" })).toHaveCount(0);
  });

  test("the Sync Manager asks for each Stick's hold and syncs only those confirmed", async ({ page }) => {
    await open(page, false);
    await page.getByRole("button", { name: "Sync Manager" }).click();
    const manager = page.getByRole("dialog", { name: "Sync Manager" });
    await manager.getByRole("checkbox", { name: "Melodic Vox", exact: true }).check();
    await manager.getByRole("checkbox", { name: "DJ STICK", exact: true }).check();
    await manager.getByRole("checkbox", { name: "TEST", exact: true }).check();
    await manager.getByRole("button", { name: "SYNC", exact: true }).click();
    // DJ STICK first: held for. TEST: cancelled.
    await expect(writeDialog(page)).toContainText("Sync 1 playlist to DJ STICK");
    await hold(page, 0);
    await expect(writeDialog(page)).toContainText("Sync 1 playlist to TEST", { timeout: 4000 });
    await page.mouse.up();
    await writeDialog(page).getByRole("button", { name: "Cancel" }).click();
    await expect(manager.getByRole("status")).toContainText("TEST: Not written: the sync to this Stick was not confirmed.");
    const synced = await page.evaluate(() => (window as unknown as Hooks).__stickWrites().map((w) => [w.method, w.path]));
    expect(synced).toEqual([["syncDevices", "/Volumes/DJ STICK"]]);
  });
});

