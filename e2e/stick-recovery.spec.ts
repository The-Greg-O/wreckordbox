/**
 * A Stick whose last write did not finish (wbx-45.5), against the mock's TEST
 * stick with an interrupted export on it from the start.
 *
 * Merely looking at a Stick never settles that write: every read reports it
 * and the window shows it in place of the Stick's playlists. Settling is an
 * action the DJ chooses in the Stick's panel, named for the Stick and saying
 * what it does. `__stickSettles` counts the mock's settle calls, the only
 * call that writes the Stick here.
 */
import { expect, test, type Page } from "@playwright/test";
import { confirmStickWrite, enableViewShell } from "./helpers";

const TEST = "/Volumes/TEST";
const NOTICE = "A write to this Stick did not finish";
const ACTION = "Finish or roll back the unfinished write on TEST";

async function interrupted(page: Page) {
  await page.addInitScript((path) => {
    (window as unknown as { __initialUnfinishedWrites: string[] }).__initialUnfinishedWrites = [path];
  }, TEST);
}

const settles = (page: Page) =>
  page.evaluate((path) => (window as unknown as { __stickSettles: (p: string) => number }).__stickSettles(path), TEST);

test.describe("the old layout", () => {
  async function devices(page: Page) {
    await interrupted(page);
    await page.goto("/?writable=1");
    await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
    await page.getByRole("tablist", { name: "Library sources" }).getByRole("tab", { name: "Devices" }).click();
  }

  test("expanding the Stick, and later listing refreshes, show the unfinished write instead of its playlists and settle nothing", async ({ page }) => {
    await devices(page);
    await page.getByRole("button", { name: "Expand TEST" }).dispatchEvent("mousedown");
    await expect(page.getByRole("treeitem", { name: NOTICE })).toBeVisible();
    await expect(page.locator('[role="treeitem"][data-kind="deviceLibrary"]')).toHaveCount(0);
    // The window asks for the device list again when it gains the focus,
    // and reads every Stick already opened.
    await page.evaluate(() => window.dispatchEvent(new Event("focus")));
    await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
    await expect(page.getByRole("treeitem", { name: NOTICE })).toBeVisible();
    expect(await settles(page)).toBe(0);
  });

  test("the Stick's panel names the Stick and what settling does, and settles only when the DJ chooses it", async ({ page }) => {
    await devices(page);
    await page.getByRole("button", { name: "Expand TEST" }).dispatchEvent("mousedown");
    await page.getByRole("treeitem", { name: /TEST/ }).first().click();
    const panel = page.getByRole("region", { name: "Device TEST" });
    const notice = panel.getByRole("group", { name: "Unfinished write on TEST" });
    await expect(notice).toContainText(NOTICE);
    await expect(notice).toContainText("finishes that export");
    await expect(notice).toContainText("puts the Stick back");
    await expect(panel.getByRole("tablist", { name: "Device settings" })).toHaveCount(0);
    expect(await settles(page)).toBe(0);

    await notice.getByRole("button", { name: ACTION }).click();
    // Settling writes to the Stick: staged and held for like every Stick write (wbx-45.4).
    await confirmStickWrite(page);

    await expect.poll(() => settles(page)).toBe(1);
    await expect(panel.getByRole("tablist", { name: "Device settings" })).toBeVisible();
    await expect(notice).toHaveCount(0);
    await expect(page.locator('[role="treeitem"][data-kind="deviceLibrary"]')).toHaveCount(2);
    await expect(page.getByRole("treeitem", { name: NOTICE })).toHaveCount(0);
    expect(await settles(page)).toBe(1);
  });

  test("a playlist opened after the Stick's write was interrupted reads nothing from it and settles nothing", async ({ page }) => {
    await page.goto("/?writable=1");
    await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
    await page.getByRole("tablist", { name: "Library sources" }).getByRole("tab", { name: "Devices" }).click();
    await page.getByRole("button", { name: "Expand TEST" }).dispatchEvent("mousedown");
    await expect(page.locator('[role="treeitem"][data-kind="deviceLibrary"]')).toHaveCount(2);
    // An export to it stops part way, behind the window's back.
    await page.evaluate((path) => (window as unknown as { __interruptStickWrite: (p: string) => void }).__interruptStickWrite(path), TEST);

    await page.locator('[role="treeitem"][data-kind="devicePlaylist"]').filter({ hasText: "Melodic Vox" }).first().click();

    await expect(page.getByRole("row").filter({ has: page.getByRole("gridcell") })).toHaveCount(0);
    expect(await settles(page)).toBe(0);
  });
});

test("an answer for a Stick pulled out while it was read is dropped, so the Stick put back in its place shows its own state", async ({ page }) => {
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await page.getByRole("tablist", { name: "Library sources" }).getByRole("tab", { name: "Devices" }).click();
  await page.getByRole("button", { name: "Expand TEST" }).dispatchEvent("mousedown");
  await expect(page.locator('[role="treeitem"][data-kind="deviceLibrary"]')).toHaveCount(2);
  const hooks = (name: string, path: string) => page.evaluate(([n, p]) => (window as unknown as Record<string, (p: string) => void>)[n as string]!(p as string), [name, path]);
  const refresh = () => page.evaluate(() => window.dispatchEvent(new Event("focus")));

  // The next read of TEST is held; a refresh starts it.
  await hooks("__holdDeviceLibraries", TEST);
  await refresh();
  await expect.poll(() => page.evaluate((p) => (window as unknown as { __deviceLibrariesHeld: (p: string) => number }).__deviceLibrariesHeld(p), TEST)).toBe(1);
  // TEST is pulled out, and a Stick with an unfinished write goes in at the same place.
  await hooks("__unmountDevice", TEST);
  await refresh();
  await expect(page.getByRole("treeitem", { name: /TEST/ })).toHaveCount(0);
  await hooks("__interruptStickWrite", TEST);
  await hooks("__remountDevice", TEST);
  await refresh();
  await expect(page.getByRole("treeitem", { name: /TEST/ })).toHaveCount(1);

  // The old Stick's answer arrives: it must never be shown for the new one,
  // not even for a moment.
  await page.evaluate(() => {
    const seen = window as unknown as { __sawStaleLibraries: boolean };
    seen.__sawStaleLibraries = false;
    const look = () => {
      if (document.querySelector('[role="treeitem"][data-kind="deviceLibrary"]')) seen.__sawStaleLibraries = true;
    };
    new MutationObserver(look).observe(document.body, { childList: true, subtree: true });
  });
  await hooks("__releaseDeviceLibraries", TEST);
  await page.waitForTimeout(1000);
  expect(await page.evaluate(() => (window as unknown as { __sawStaleLibraries: boolean }).__sawStaleLibraries)).toBe(false);
  // Opened again, the Stick in its place shows its own state.
  await page.getByRole("button", { name: "Collapse TEST" }).dispatchEvent("mousedown");
  await page.getByRole("button", { name: "Expand TEST" }).dispatchEvent("mousedown");
  await expect(page.getByRole("treeitem", { name: NOTICE })).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __sawStaleLibraries: boolean }).__sawStaleLibraries)).toBe(false);
  expect(await settles(page)).toBe(0);
});

test("opening the panel of a Stick with an export but no DEVSETTING.DAT shows the DJ System defaults and writes nothing", async ({ page }) => {
  await page.addInitScript((path) => {
    (window as unknown as { __initialNoDevSetting: string[] }).__initialNoDevSetting = [path];
    window.localStorage.setItem("rbl.preferences", JSON.stringify({ djSystem: { waveformColor: "rgb" } }));
  }, TEST);
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await page.getByRole("tablist", { name: "Library sources" }).getByRole("tab", { name: "Devices" }).click();
  await page.getByRole("treeitem", { name: /TEST/ }).first().click();
  const panel = page.getByRole("region", { name: "Device TEST" });
  await expect(panel.getByRole("radio", { name: "RGB" })).toBeChecked();
  const writes = () => page.evaluate((path) => (window as unknown as { __stickSettingsWrites: (p: string) => number }).__stickSettingsWrites(path), TEST);
  expect(await writes()).toBe(0);
  // Choosing a setting is the DJ's write: then the Stick gets its file.
  await panel.getByRole("radio", { name: "BLUE", exact: true }).check();
  // Staged and held for, as every Stick write is (wbx-45.4).
  await confirmStickWrite(page);
  await expect.poll(writes).toBe(1);
});

test.describe("the view shell", () => {
  const tree = (page: Page) => page.getByTestId("shell-tree").getByRole("tree", { name: "Tree" });
  const stick = (page: Page) => tree(page).getByRole("group", { name: "Stick TEST", exact: true });

  test("the Stick shows its unfinished write; its view offers the named action, which settles it and lists its playlists", async ({ page }) => {
    await interrupted(page);
    await enableViewShell(page);
    await page.goto("/");
    await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
    await expect(stick(page)).toContainText(NOTICE);

    await stick(page).getByRole("treeitem", { name: "TEST", exact: true }).click();
    const working = page.getByTestId("shell-working");
    const notice = working.getByRole("group", { name: "Unfinished write on TEST" });
    await expect(notice).toContainText(NOTICE);
    expect(await settles(page)).toBe(0);

    await notice.getByRole("button", { name: ACTION }).click();
    // Settling writes to the Stick: staged and held for like every Stick write (wbx-45.4).
    await confirmStickWrite(page);

    await expect.poll(() => settles(page)).toBe(1);
    await expect(stick(page)).not.toContainText(NOTICE);
    await expect(stick(page).getByRole("treeitem", { name: "Melodic Vox" })).toBeVisible();
    await expect(working.getByRole("tablist", { name: "Device settings" })).toBeVisible();
  });
});
