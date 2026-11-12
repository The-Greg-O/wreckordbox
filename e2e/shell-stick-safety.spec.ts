/**
 * With the view shell on, nothing settles an unfinished write on a Stick
 * unless the DJ asks (wbx-45.4 review, findings 6 and 7; wbx-45.5).
 *
 * The mock records every call that, in the real backend, runs
 * `rbl_export::recover()` first (`__stickRecoveringCalls`), and when such a
 * call reaches a Stick marked as holding an unfinished write it logs the
 * Stick as settled (`__stickRecovered`). Every test here ends with nothing
 * settled.
 */
import { expect, test, type Page } from "@playwright/test";

const TEST = "/Volumes/TEST";
const tree = (page: Page) => page.getByTestId("shell-tree").getByRole("tree", { name: "Tree" });
const stick = (page: Page) => tree(page).getByRole("group", { name: "Stick TEST", exact: true });
const inPlaylists = (page: Page, name: string) =>
  tree(page).getByRole("group", { name: "Playlists", exact: true }).getByRole("treeitem", { name, exact: true });

type Hooks = {
  __stickRecovered: () => string[];
  __stickRecoveringCalls: () => { method: string; path: string }[];
  __stickWrites: () => { method: string; path: string }[];
  __setStickPending: (path: string, pending: boolean) => void;
  __failNextStickWrite: (path: string) => void;
  __unmountDevice: (path: string) => void;
  __remountDevice: (path: string) => void;
};
const recovered = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__stickRecovered());
const writes = (page: Page) => page.evaluate(() => (window as unknown as Hooks).__stickWrites());
const setPending = (page: Page) => page.evaluate((path) => {
  (window as unknown as Hooks).__setStickPending(path, true);
}, TEST);
/** TEST pulled out and put back holding an unfinished write: a new mount, which the tree reads afresh. */
const remountPending = async (page: Page) => {
  await page.evaluate((path) => {
    (window as unknown as Hooks).__unmountDevice(path);
    window.dispatchEvent(new Event("focus"));
  }, TEST);
  await expect(stick(page)).toHaveCount(0);
  await page.evaluate((path) => {
    (window as unknown as Hooks).__setStickPending(path, true);
    (window as unknown as Hooks).__remountDevice(path);
    window.dispatchEvent(new Event("focus"));
  }, TEST);
  await expect(stick(page)).toContainText("A write to this Stick did not finish");
};
const refocus = (page: Page) => page.evaluate(() => window.dispatchEvent(new Event("focus")));

function seed(page: Page, shell: boolean) {
  return page.addInitScript((viewShell) => {
    if (sessionStorage.getItem("seeded") === "1") return;
    sessionStorage.setItem("seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify({ advanced: { protectLibrary: false }, view: { viewShell } }));
  }, shell);
}

/** The view shell switched on from another window, as the Preferences window would. */
async function switchToShell(page: Page) {
  await page.evaluate(() => {
    const prefs = JSON.parse(localStorage.getItem("rbl.preferences") ?? "{}") as { view?: Record<string, unknown> };
    prefs.view = { ...prefs.view, viewShell: true };
    localStorage.setItem("rbl.preferences", JSON.stringify(prefs));
    window.dispatchEvent(new StorageEvent("storage", { key: "rbl.preferences" }));
  });
  await expect(page.getByTestId("shell")).toBeVisible();
}

test("a Stick opened in the old layout is not read again through the recovering path once the shell is on", async ({ page }) => {
  await seed(page, false);
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await page.getByRole("tablist", { name: "Library sources" }).getByRole("tab", { name: "Devices" }).click();
  await page.getByRole("button", { name: "Expand TEST" }).dispatchEvent("mousedown");
  await expect(page.locator('[role="treeitem"][data-kind="deviceLibrary"]')).toHaveCount(2);

  await switchToShell(page);
  // TEST now holds an unfinished write (left by a crash, say). The device
  // list changes when another Stick comes and goes, and on every focus.
  await setPending(page);
  await page.evaluate(() => {
    (window as unknown as Hooks).__unmountDevice("/Volumes/DJ STICK");
    window.dispatchEvent(new Event("focus"));
  });
  await expect(tree(page).getByRole("group", { name: "Stick DJ STICK", exact: true })).toHaveCount(0);
  await page.evaluate(() => {
    (window as unknown as Hooks).__remountDevice("/Volumes/DJ STICK");
    window.dispatchEvent(new Event("focus"));
  });
  await expect(tree(page).getByRole("group", { name: "Stick DJ STICK", exact: true })).toBeVisible();
  await refocus(page);
  await page.waitForTimeout(500);
  expect(await recovered(page)).toEqual([]);
});

test("a Stick playlist left selected is not reopened through the recovering path after a restart", async ({ page }) => {
  await seed(page, true);
  await page.goto("/?writable=1");
  await stick(page).getByRole("treeitem", { name: "Melodic Vox" }).click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await setPending(page);
  await page.reload();
  await expect(tree(page)).toBeVisible();
  await expect(stick(page)).toContainText("A write to this Stick did not finish");
  await page.waitForTimeout(300);
  expect(await recovered(page)).toEqual([]);
});

test("a Stick with an unfinished write is offered to no export, takes no drop, and its playlists are not opened", async ({ page }) => {
  await seed(page, true);
  await page.goto("/?writable=1");
  await expect(stick(page).getByRole("treeitem", { name: "Melodic Vox" })).toBeVisible();
  await remountPending(page);

  await inPlaylists(page, "Hardstyle").click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Playlist", exact: true });
  await menu.getByRole("menuitem", { name: "Export Playlist" }).hover();
  await expect(page.getByRole("menuitem", { name: "TEST", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");

  expect(await writes(page)).toEqual([]);
  expect(await recovered(page)).toEqual([]);
});

test("a Stick write that fails part-way leaves the Stick refused until it is settled", async ({ page }) => {
  await seed(page, true);
  await page.goto("/?writable=1");
  await expect(stick(page).getByRole("treeitem", { name: "Melodic Vox" })).toBeVisible();
  await page.evaluate((path) => { (window as unknown as Hooks).__failNextStickWrite(path); }, TEST);
  // Create New Playlist on the Stick, confirmed with the hold.
  await stick(page).getByRole("treeitem", { name: "TEST", exact: true }).click({ button: "right" });
  await page.getByRole("menu", { name: "Playlists" }).getByRole("menuitem", { name: "Create New Playlist" }).click();
  const confirm = page.getByRole("alertdialog", { name: /Write to TEST/ });
  await confirm.getByRole("button", { name: /Hold to write/ }).dispatchEvent("pointerdown");
  await page.waitForTimeout(2300);
  await confirm.getByRole("button", { name: /Hold to write/ }).dispatchEvent("pointerup");
  await expect(page.getByTestId("shell-status")).toContainText("stopped part-way");
  // The tree looks at the Stick again and shows what is there now.
  await expect(stick(page)).toContainText("A write to this Stick did not finish");
  const after = (await writes(page)).length;
  await inPlaylists(page, "Hardstyle").click({ button: "right" });
  await page.getByRole("menu", { name: "Playlist", exact: true }).getByRole("menuitem", { name: "Export Playlist" }).hover();
  await expect(page.getByRole("menuitem", { name: "TEST", exact: true })).toBeDisabled();
  expect((await writes(page)).length).toBe(after);
  expect(await recovered(page)).toEqual([]);
});
