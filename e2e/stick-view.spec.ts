/**
 * The Stick view (wbx-71): the mounted Stick, Gig or Spare, protected or
 * unlocked, its last load and its space before and after; the playlists a
 * load includes; what the load will do by kind of change, with every change
 * one press away; too little space said and the load disabled; no Stick
 * said. Showing the view, choosing and the dry run write nothing; the load
 * goes through the window's Stick write gate (plan, Stick Protection, hold).
 *
 * Mock hooks: `src/ipc/mock-stick-load.ts` and `mock-stick-safety.ts`.
 */
import { expect, test, type Page } from "@playwright/test";

import { open, pauseButton, player, titles } from "./compactPlayer";
import { confirmStickWrite } from "./helpers";

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const view = (page: Page) => page.getByTestId("stick-view");
const choose = (page: Page) => view(page).getByTestId("stick-choose");
const loadButton = (page: Page) => view(page).getByTestId("stick-load");

const DJ = "/Volumes/DJ STICK";
const TEST = "/Volumes/TEST";

async function call<T>(page: Page, name: string, ...args: unknown[]): Promise<T> {
  return page.evaluate(([hook, rest]) => {
    const hooks = window as unknown as Record<string, (...a: unknown[]) => T>;
    return hooks[hook]!(...rest);
  }, [name, args] as [string, unknown[]]);
}

const writes = (page: Page) => call<{ method: string; path: string }[]>(page, "__stickWrites");
const dryRuns = async (page: Page) => (await call<{ dryRuns: number }>(page, "__stickLoadReads")).dryRuns;

async function openStick(page: Page): Promise<void> {
  await open(page);
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-name")).toBeVisible();
}

/** The playlists offered (not folders), in tree order. */
const playlistRows = (page: Page) => choose(page).locator("label").filter({ hasNotText: "(folder" });

/** Ticks the first playlist offered; answers its name. */
async function chooseFirst(page: Page): Promise<string> {
  const row = playlistRows(page).first();
  const label = (await row.innerText()).trim();
  await row.getByRole("checkbox").check();
  return label;
}

test("the view shows the Stick's name, kind, protection, last load and space; choosing and the dry run write nothing", async ({ page }) => {
  await openStick(page);

  await expect(view(page).getByTestId("stick-name")).toHaveText("DJ STICK");
  await expect(view(page).getByTestId("stick-kind")).toHaveText("Gig stick");
  await expect(view(page).getByTestId("stick-protection")).toHaveText("Protected");
  await expect(view(page).getByTestId("stick-meta")).toContainText("Never loaded by wreckordbox");
  await expect(view(page).getByTestId("stick-space-line")).toHaveText("Free now 25.8 GB of 34.4 GB");
  // Nothing chosen yet: the view asks for playlists and the load waits.
  await expect(view(page).getByTestId("stick-summary-none")).toBeVisible();
  await expect(loadButton(page)).toBeDisabled();
  await expect(view(page).getByTestId("stick-load-why")).toHaveText("Choose at least one playlist to load.");

  const name = await chooseFirst(page);
  const summary = view(page).getByTestId("stick-summary");
  await expect(summary.locator('[data-change="added"]')).toContainText(`tracks added, across ${name}`);
  await expect(summary.locator('[data-change="playlists"]')).toHaveText(/^1\s*playlist created$/);
  // Space before and after, from the dry run.
  await expect(view(page).getByTestId("stick-space-line")).toHaveText(/^Free now 25\.8 GB of 34\.4 GB · after this load 25\.\d GB$/);
  await expect(loadButton(page)).toBeEnabled();
  expect(await writes(page)).toEqual([]);
  expect(await call(page, "__stickRecoveringCalls")).toEqual([]);
});

test("a Spare stick is shown as one, and another mounted Stick can be chosen", async ({ page }) => {
  await open(page);
  await call(page, "__setStickSpare", TEST, true);
  await tab(page, "Stick").click();
  await view(page).getByRole("group", { name: "Mounted Sticks" }).getByRole("button", { name: "TEST" }).click();
  await expect(view(page).getByTestId("stick-name")).toHaveText("TEST");
  await expect(view(page).getByTestId("stick-kind")).toHaveText("Spare stick");
  await expect(view(page)).toContainText("A Spare stick takes a 1 s hold.");
});

test("the DJ chooses the playlists, the summary follows, and every change opens from it", async ({ page }) => {
  await openStick(page);
  const boxes = playlistRows(page).getByRole("checkbox");
  await boxes.nth(0).check();
  await expect(view(page).getByTestId("stick-summary").locator('[data-change="playlists"]')).toHaveText(/^1\s*playlist created$/);
  await boxes.nth(1).check();
  await expect(view(page).getByTestId("stick-summary").locator('[data-change="playlists"]')).toHaveText(/^2\s*playlists created$/);
  const asked = await call<{ playlists: string[] }>(page, "__stickLoadLastDryRun");
  expect(asked.playlists).toHaveLength(2);

  const show = view(page).getByRole("button", { name: "Show every change" });
  await show.click();
  const changes = view(page).getByRole("region", { name: "Every change" });
  await expect(changes).toBeVisible();
  await expect(changes.getByRole("heading", { name: /^Tracks added \(\d+\)$/ })).toBeVisible();
  await expect(changes.getByRole("heading", { name: "Playlists created (2)" })).toBeVisible();
  const added = Number(/\((\d+)\)/.exec(await changes.getByRole("heading", { name: /^Tracks added/ }).innerText())?.[1]);
  await expect(changes.getByRole("heading", { name: /^Tracks added/ }).locator("xpath=..").getByRole("listitem")).toHaveCount(added);
  await expect(view(page).getByTestId("stick-summary").locator('[data-change="added"]')).toContainText(`+${added}`);

  // Unticking one takes it out of the next dry run.
  await boxes.nth(1).uncheck();
  await expect(view(page).getByTestId("stick-summary").locator('[data-change="playlists"]')).toHaveText(/^1\s*playlist created$/);
  expect(await writes(page)).toEqual([]);
});

test("a Stick with too little space says by how much and the load is disabled", async ({ page }) => {
  await open(page);
  await call(page, "__setStickFree", DJ, 1_000_000);
  await tab(page, "Stick").click();
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-shortfall")).toHaveText(/^Not enough space: this load needs \d+(\.\d)? (MB|GB) more than the Stick has free\.$/);
  await expect(loadButton(page)).toBeDisabled();
  await expect(view(page).getByTestId("stick-load-why")).toContainText("Not enough space");
  expect(await writes(page)).toEqual([]);
});

test("with no Stick mounted the view says so", async ({ page }) => {
  await open(page);
  await page.evaluate(([a, b]) => {
    const hooks = window as unknown as { __unmountDevice: (path: string) => void };
    hooks.__unmountDevice(a!);
    hooks.__unmountDevice(b!);
    window.dispatchEvent(new Event("focus"));
  }, [DJ, TEST]);
  await expect(page.getByTestId("shell-tree").getByRole("group", { name: "Stick TEST" })).toHaveCount(0);
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-none")).toContainText("No Stick is mounted");
});

test("a load is staged through the hold: cancelling writes nothing; holding unlocks the Gig stick by name and loads it", async ({ page }) => {
  await openStick(page);
  const name = await chooseFirst(page);
  await expect(loadButton(page)).toBeEnabled();

  // The gate's dialog: which Stick, Gig, protected; Cancel writes nothing.
  await loadButton(page).click();
  const dialog = page.getByRole("alertdialog", { name: "Write to DJ STICK" });
  await expect(dialog).toContainText("DJ STICK · Gig stick");
  await expect(dialog).toContainText("Load 1 playlist onto DJ STICK");
  await expect(dialog.getByRole("button", { name: /Hold to write \(2 s\)/ })).toBeDisabled();
  await page.waitForTimeout(450);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await writes(page)).toEqual([]);
  expect(await call(page, "__stickProtected", DJ)).toBe(true);

  await loadButton(page).click();
  await confirmStickWrite(page);
  await expect(view(page).getByTestId("stick-outcome")).toHaveAttribute("data-outcome", "loaded");
  await expect(view(page).getByTestId("stick-outcome")).toContainText("Loaded: DJ STICK holds");
  expect((await writes(page)).map((w) => [w.method, w.path])).toEqual([["stickViewLoad", DJ]]);
  expect((await call<{ playlists: string[] }>(page, "__stickHeld", DJ)).playlists).toEqual([name]);
  // Read again after the load: unlocked, last loaded today, nothing left to do.
  await expect(view(page).getByTestId("stick-protection")).toHaveText("Unlocked");
  await expect(view(page).getByTestId("stick-meta")).toContainText(/Last loaded [A-Z][a-z]{2} \d\d/);
  await expect(view(page).getByTestId("stick-summary")).toHaveText(/Nothing to change/);
});

test("a load without the hold is refused by the backend and writes nothing", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await expect(loadButton(page)).toBeEnabled();
  await call(page, "__setStickUnlocked", DJ, true);
  const last = await call<{ stick: string; playlists: string[] }>(page, "__stickLoadLastDryRun");
  // A script calling the load straight, with the dry run's plan but no token.
  const refused = await page.evaluate(async ([stick, playlists]) => {
    const write = (window as unknown as { __stickWriteUnstaged: (name: string, ...a: unknown[]) => Promise<unknown> }).__stickWriteUnstaged;
    try {
      await write("stickViewLoad", stick, playlists, "mock-load-1");
      return "loaded";
    } catch (error) {
      return (error as Error).message;
    }
  }, [last.stick, last.playlists] as const);
  expect(refused).toContain("This write to the Stick was not confirmed");
  expect(await writes(page)).toEqual([]);
});

test("a Stick the service cannot plan for says why, and a failed dry run is shown, not the one before", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();

  await call(page, "__failStickDryRuns", "The Stick could not be read.");
  await playlistRows(page).nth(1).getByRole("checkbox").check();
  await expect(view(page).getByTestId("stick-dry-run-failed")).toContainText("The Stick could not be read.");
  await expect(view(page).getByTestId("stick-summary")).toHaveCount(0);
  await expect(loadButton(page)).toBeDisabled();

  await call(page, "__failStickDryRuns", null);
  await call(page, "__setStickNotLoadable", DJ, "Its database is in WAL mode, so what the export would copy cannot be read without writing to it.");
  await view(page).getByRole("button", { name: "Try again" }).click();
  await expect(view(page).getByTestId("stick-not-loadable")).toHaveText(/^This Stick cannot be loaded: Its database is in WAL mode/);
  await expect(loadButton(page)).toBeDisabled();
  expect(await writes(page)).toEqual([]);
});

test("a burst of Library changes costs at most two more dry runs", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  await expect.poll(() => dryRuns(page)).toBeGreaterThan(0);
  await call(page, "__holdStickDryRuns");
  const before = await dryRuns(page);
  await call(page, "__emitLibraryChanged", 30);
  await page.waitForTimeout(300);
  await call(page, "__releaseStickDryRuns");
  await page.waitForTimeout(500);
  expect(await dryRuns(page) - before).toBeLessThanOrEqual(2);
  expect(await dryRuns(page) - before).toBeGreaterThanOrEqual(1);
});

test("the view's controls work from the keyboard and Space never reaches the deck", async ({ page }) => {
  await open(page);
  await titles(page).nth(3).dblclick();
  await tab(page, "Stick").click();
  const box = playlistRows(page).first().getByRole("checkbox");
  await box.focus();
  await page.keyboard.press("Space");
  await expect(box).toBeChecked();
  const show = view(page).getByRole("button", { name: "Show every change" });
  await show.focus();
  await page.keyboard.press("Enter");
  await expect(view(page).getByRole("region", { name: "Every change" })).toBeVisible();
  await view(page).getByRole("button", { name: "Hide the changes" }).focus();
  await page.keyboard.press("Space");
  await expect(view(page).getByRole("region", { name: "Every change" })).toHaveCount(0);
  await expect(pauseButton(page)).toHaveCount(0);
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();
});

for (const width of [1280, 900, 640, 436]) {
  test(`at ${width} px the view fits the window without sideways scrolling`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await openStick(page);
    await chooseFirst(page);
    await expect(view(page).getByTestId("stick-summary")).toBeVisible();
    const fits = await view(page).evaluate((el) => el.scrollWidth <= el.clientWidth + 1);
    expect(fits).toBe(true);
    const box = await loadButton(page).boundingBox();
    expect(box).not.toBeNull();
    expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(width);
    await page.screenshot({ path: `test-results/stick-view-${width}.png` });
  });
}
