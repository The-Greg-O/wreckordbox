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
import { enableViewShell } from "./helpers";

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

    await expect.poll(() => settles(page)).toBe(1);
    await expect(stick(page)).not.toContainText(NOTICE);
    await expect(stick(page).getByRole("treeitem", { name: "Melodic Vox" })).toBeVisible();
    await expect(working.getByRole("tablist", { name: "Device settings" })).toBeVisible();
  });
});
