/**
 * Hold to load (wbx-72): a load starts only when the DJ holds the Stick
 * view's big button (or `L`) for the whole hold, longer for a Gig stick
 * than a Spare stick; a short press or an early release writes nothing.
 * While it runs it shows a plain progress bar naming the track being
 * written; `Esc` shrinks it to the status line, where it keeps running
 * while other views are used, and a click there brings it back. It ends
 * with what it wrote and an Eject; "safe to remove" only after the eject.
 *
 * Mock hooks: `src/ipc/mock-stick-load.ts` and `mock-stick-safety.ts`.
 */
import { expect, test, type Page } from "@playwright/test";

import { open, pauseButton, player, titles } from "./compactPlayer";

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const view = (page: Page) => page.getByTestId("stick-view");
const holdButton = (page: Page) => view(page).getByTestId("stick-load");
const progress = (page: Page) => view(page).getByTestId("stick-progress");
const statusLoad = (page: Page) => page.getByTestId("status-stick-load");

const DJ = "/Volumes/DJ STICK";
const TEST = "/Volumes/TEST";

async function call<T>(page: Page, name: string, ...args: unknown[]): Promise<T> {
  return page.evaluate(([hook, rest]) => {
    const hooks = window as unknown as Record<string, (...a: unknown[]) => T>;
    return hooks[hook]!(...rest);
  }, [name, args] as [string, unknown[]]);
}

const writes = (page: Page) => call<{ method: string; path: string }[]>(page, "__stickWrites");
const loadCalls = (page: Page) => call<unknown[]>(page, "__stickLoadCalls");

/** The Stick view with the first playlist chosen and its summary read; the Gig stick unlocked by name. */
async function ready(page: Page, stick: "DJ STICK" | "TEST" = "DJ STICK"): Promise<void> {
  await open(page);
  if (stick === "TEST") await call(page, "__setStickSpare", TEST, true);
  await tab(page, "Stick").click();
  if (stick === "TEST") {
    await view(page).getByRole("group", { name: "Mounted Sticks" }).getByRole("button", { name: "TEST" }).click();
  }
  await expect(view(page).getByTestId("stick-name")).toHaveText(stick);
  await view(page).getByTestId("stick-choose").locator("label").filter({ hasNotText: "(folder" }).first().getByRole("checkbox").check();
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  const field = view(page).getByRole("textbox", { name: `Type ${stick} to unlock it` });
  if (await field.count() > 0) await field.fill(stick);
  await view(page).getByRole("button", { name: `Unlock ${stick}` }).click();
  await expect(view(page).getByTestId("stick-protection")).toHaveText("Unlocked");
  await expect(holdButton(page)).toBeEnabled();
}

/** Presses the big button with the mouse for `ms`, then lets go. */
async function pressFor(page: Page, ms: number): Promise<void> {
  await holdButton(page).hover();
  await page.mouse.down();
  await page.waitForTimeout(ms);
  await page.mouse.up();
}

test("a short press does nothing; letting go before the hold is full writes nothing", async ({ page }) => {
  await ready(page);
  await expect(holdButton(page)).toContainText("Hold to load");
  await expect(holdButton(page)).toContainText("2 s · Gig stick");
  await pressFor(page, 150);
  await pressFor(page, 1200);
  await page.waitForTimeout(1200);
  expect(await loadCalls(page)).toEqual([]);
  expect(await writes(page)).toEqual([]);
  await expect(holdButton(page)).not.toHaveAttribute("data-holding");
  await expect(progress(page)).toHaveCount(0);
});

test("a full hold starts the load: a plain bar names the track being written, and it ends with what it wrote", async ({ page }) => {
  await ready(page);
  await call(page, "__holdStickLoads");
  await pressFor(page, 2300);
  await expect(progress(page)).toBeVisible();
  await expect(progress(page)).toContainText("Loading DJ STICK");
  await expect(progress(page).getByRole("progressbar")).toBeVisible();
  await expect(progress(page)).toContainText(/Copying 0 of \d+ · \S/);
  expect(await loadCalls(page)).toHaveLength(1);

  await call(page, "__releaseStickLoads");
  await expect(progress(page)).toHaveCount(0);
  const done = view(page).getByTestId("stick-finished");
  // Finished is not "safe to remove": that needs the eject (review, wbx-72).
  await expect(done).toContainText("Loaded DJ STICK");
  await expect(done).not.toContainText("safe to remove");
  await expect(done.getByTestId("stick-outcome")).toContainText(/Written: \d+ tracks, 1 playlist/);
  expect((await writes(page)).map((w) => [w.method, w.path])).toEqual([["stickViewLoad", DJ]]);
  // Done: back to the button, nothing more written.
  await done.getByRole("button", { name: "Done" }).click();
  await expect(done).toHaveCount(0);
  await expect(holdButton(page)).toBeVisible();
  expect(await writes(page)).toHaveLength(1);
});

test("a Gig stick needs a longer hold than a Spare stick", async ({ page }) => {
  await ready(page);
  await pressFor(page, 1400);
  await page.waitForTimeout(800);
  expect(await loadCalls(page)).toEqual([]);

  await view(page).getByRole("group", { name: "Mounted Sticks" }).getByRole("button", { name: "TEST" }).click();
  await call(page, "__setStickSpare", TEST, true);
  await page.reload();
  await ready(page, "TEST");
  await expect(holdButton(page)).toContainText("1 s · Spare stick");
  await pressFor(page, 1400);
  await expect(view(page).getByTestId("stick-finished")).toContainText("Loaded TEST");
  expect((await writes(page)).map((w) => w.path)).toEqual([TEST]);
});

test("holding L loads; a tap of L, L in a text field, L with a dialog open and an L held from before the view appeared do not", async ({ page }) => {
  await ready(page);
  // A tap.
  await page.keyboard.press("l");
  await page.waitForTimeout(300);
  // In a text field: the field's key (a field of the view's own, as a name or filter would be).
  await page.evaluate(() => {
    const field = document.createElement("input");
    field.setAttribute("aria-label", "A text field");
    document.querySelector('[data-testid="stick-view"] section')!.append(field);
  });
  const field = view(page).getByRole("textbox", { name: "A text field" });
  await field.focus();
  await page.keyboard.down("l");
  await page.waitForTimeout(2300);
  await page.keyboard.up("l");
  await expect(field).toHaveValue(/^l+$/);
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  // With the command palette open.
  await page.keyboard.press("Control+Shift+P");
  await expect(page.locator("[data-command-palette]")).toBeVisible();
  await page.keyboard.down("l");
  await page.waitForTimeout(2300);
  await page.keyboard.up("l");
  await page.keyboard.press("Escape");
  await expect(page.locator("[data-command-palette]")).toHaveCount(0);
  // Held down in the Library view, still held when the Stick view appears: only repeats reach it.
  await tab(page, "Library").click();
  await page.keyboard.down("l");
  await tab(page, "Stick").click();
  await expect(holdButton(page)).toBeEnabled();
  for (let at = 0; at < 14; at += 1) {
    await page.keyboard.down("l");
    await page.waitForTimeout(200);
  }
  await page.keyboard.up("l");
  await page.waitForTimeout(300);
  expect(await loadCalls(page)).toEqual([]);
  expect(await writes(page)).toEqual([]);

  // A fresh, full hold of L.
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.down("l");
  await page.waitForTimeout(2300);
  await page.keyboard.up("l");
  await expect(view(page).getByTestId("stick-finished")).toContainText("Loaded DJ STICK");
  expect((await writes(page)).map((w) => w.path)).toEqual([DJ]);
});

test("the load shrinks to the status line with Esc, keeps running while another view is used, and comes back from the status line", async ({ page }) => {
  await ready(page);
  await call(page, "__holdStickLoads");
  await pressFor(page, 2300);
  await expect(progress(page)).toBeVisible();
  await expect(statusLoad(page)).toContainText("Loading DJ STICK");

  await page.keyboard.press("Escape");
  await expect(progress(page)).toHaveCount(0);
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  await expect(statusLoad(page)).toContainText("Loading DJ STICK");
  await expect(statusLoad(page).getByRole("progressbar")).toBeVisible();

  await tab(page, "Library").click();
  await expect(statusLoad(page)).toContainText("Loading DJ STICK");
  await titles(page).first().click();
  // The load goes on: a burst of progress reaches the status line, the latest shown.
  await call(page, "__emitStickLoadProgress", DJ, 300);
  await expect(statusLoad(page)).toContainText("300/300");

  await statusLoad(page).click();
  await expect(tab(page, "Stick")).toHaveAttribute("aria-selected", "true");
  await expect(progress(page)).toBeVisible();
  await expect(progress(page)).toContainText("Copying 300 of 300 · Burst track 300");

  await call(page, "__releaseStickLoads");
  await expect(view(page).getByTestId("stick-finished")).toContainText("Loaded DJ STICK");
  await expect(statusLoad(page)).toHaveCount(0);
});

test("the hold button is held with Space from the keyboard, and Space never reaches the deck", async ({ page }) => {
  await open(page);
  await titles(page).nth(3).dblclick();
  await page.goto(page.url());
  await ready(page);
  await holdButton(page).focus();
  await page.keyboard.down(" ");
  await page.waitForTimeout(2300);
  await page.keyboard.up(" ");
  await expect(view(page).getByTestId("stick-finished")).toContainText("Loaded DJ STICK");
  await expect(pauseButton(page)).toHaveCount(0);
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();
});

test("a protected Stick cannot be held for: the button says to unlock it first", async ({ page }) => {
  await open(page);
  await tab(page, "Stick").click();
  await view(page).getByTestId("stick-choose").locator("label").filter({ hasNotText: "(folder" }).first().getByRole("checkbox").check();
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  await expect(holdButton(page)).toBeDisabled();
  await expect(view(page).getByTestId("stick-load-why")).toHaveText("Unlock DJ STICK first.");
  await page.keyboard.down("l");
  await page.waitForTimeout(2300);
  await page.keyboard.up("l");
  expect(await writes(page)).toEqual([]);
});

// ---- Review of wbx-72 -------------------------------------------------

const approvals = (page: Page) => call<string[]>(page, "__stickApprovals");

test("a plan refreshed during the hold (another client edits the Library) cancels it: nothing is approved", async ({ page }) => {
  await ready(page);
  const shown = await call<string[]>(page, "__stickLoadPlans");
  await call(page, "__holdStickDryRuns");
  await holdButton(page).hover();
  await page.mouse.down();
  await page.waitForTimeout(200);
  // Another client's edit: the same dry run asked again, its answer (plan B) held back.
  await call(page, "__emitLibraryChanged", 1);
  await page.waitForTimeout(1600);
  // Plan B arrives at 1.8 s of the 2 s hold.
  await call(page, "__releaseStickDryRuns");
  await expect.poll(async () => (await call<string[]>(page, "__stickLoadPlans")).length).toBeGreaterThan(shown.length);
  await page.waitForTimeout(800);
  await page.mouse.up();
  expect(await approvals(page)).toEqual([]);
  expect(await loadCalls(page)).toEqual([]);
  await expect(holdButton(page)).not.toHaveAttribute("data-holding");
  // A fresh, full hold approves the plan now shown, and only it.
  const now = (await call<string[]>(page, "__stickLoadPlans")).at(-1);
  await pressFor(page, 2300);
  await expect(view(page).getByTestId("stick-finished")).toBeVisible();
  expect(await approvals(page)).toEqual([now]);
});

test("an L hold stops when the palette opens or a text field takes focus, and does not come back without a fresh press", async ({ page }) => {
  await ready(page);
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  // The palette opens during the hold.
  await page.keyboard.down("l");
  await page.waitForTimeout(800);
  await page.keyboard.press("Control+Shift+P");
  await expect(page.locator("[data-command-palette]")).toBeVisible();
  await page.waitForTimeout(1800);
  await page.keyboard.press("Escape");
  await expect(page.locator("[data-command-palette]")).toHaveCount(0);
  // Back on the view, L still down: only repeats, so no new hold.
  for (let at = 0; at < 6; at += 1) {
    await page.keyboard.down("l");
    await page.waitForTimeout(200);
  }
  await page.keyboard.up("l");
  expect(await approvals(page)).toEqual([]);

  // A text field takes the focus during the hold.
  await page.evaluate(() => {
    const field = document.createElement("input");
    field.setAttribute("aria-label", "A text field");
    document.querySelector('[data-testid="stick-view"] section')!.append(field);
  });
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.down("l");
  await page.waitForTimeout(800);
  await view(page).getByRole("textbox", { name: "A text field" }).evaluate((el) => { (el as HTMLInputElement).focus(); });
  await page.waitForTimeout(1800);
  await page.keyboard.up("l");
  expect(await approvals(page)).toEqual([]);
  expect(await loadCalls(page)).toEqual([]);
  expect(await writes(page)).toEqual([]);
});

test("a load that fails while shrunk and another view shows stays in the status line, links to its report, until acknowledged", async ({ page }) => {
  await ready(page);
  await call(page, "__holdStickLoads");
  await call(page, "__failNextStickLoad", DJ, "The Stick was removed during the export.");
  await pressFor(page, 2300);
  await expect(progress(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await tab(page, "Library").click();
  await call(page, "__releaseStickLoads");
  await expect(statusLoad(page)).toHaveAttribute("data-state", "failed");
  await expect(statusLoad(page)).toContainText("DJ STICK: the load failed");
  await page.waitForTimeout(500);
  await expect(statusLoad(page)).toBeVisible();
  await statusLoad(page).click();
  await expect(tab(page, "Stick")).toHaveAttribute("aria-selected", "true");
  await expect(view(page).getByTestId("stick-load-error")).toContainText("The Stick was removed during the export.");
  // Still there until the DJ acknowledges it.
  await expect(statusLoad(page)).toBeVisible();
  await view(page).getByTestId("stick-dismiss").click();
  await expect(statusLoad(page)).toHaveCount(0);
});

test("safe to remove appears only after the eject succeeds, for that Stick; a failed load offers the eject, never safe to remove", async ({ page }) => {
  await ready(page);
  await call(page, "__failNextStickLoad", DJ, "The Stick was removed during the export.");
  await pressFor(page, 2300);
  const failed = view(page).getByTestId("stick-failed");
  await expect(failed).toContainText("The Stick was removed during the export.");
  await expect(page.getByText(/safe to remove/i)).toHaveCount(0);
  await expect(failed.getByRole("button", { name: "Eject DJ STICK" })).toBeVisible();
  await failed.getByTestId("stick-dismiss").click();

  await pressFor(page, 2300);
  const done = view(page).getByTestId("stick-finished");
  await expect(done).toContainText("Loaded DJ STICK");
  await expect(page.getByText(/safe to remove/i)).toHaveCount(0);
  await done.getByRole("button", { name: "Eject DJ STICK" }).click();
  await expect(page.getByTestId("stick-safe-to-remove")).toHaveText(/DJ STICK is safe to remove/);
  // Through the existing eject path: the Stick is gone from the window's list.
  await expect(page.getByTestId("shell-tree").getByRole("group", { name: "Stick DJ STICK" })).toHaveCount(0);
});

test("a late progress event from an earlier load never touches a later one", async ({ page }) => {
  await ready(page);
  await pressFor(page, 2300);
  await expect(view(page).getByTestId("stick-finished")).toBeVisible();
  const [first] = await call<string[]>(page, "__stickLoadOperations");
  await view(page).getByTestId("stick-finished").getByRole("button", { name: "Done" }).click();
  await view(page).getByTestId("stick-choose").locator("label").filter({ hasNotText: "(folder" }).nth(1).getByRole("checkbox").check();
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  // Let the reads after the first load settle: a plan refreshed mid-hold would (rightly) cancel it.
  await page.waitForTimeout(1000);
  await call(page, "__holdStickLoads");
  await pressFor(page, 2300);
  await expect(progress(page)).toBeVisible();
  await expect(progress(page)).toContainText(/Copying 0 of/);
  // Load A's late burst.
  await call(page, "__emitStickLoadProgress", DJ, 50, first);
  await page.waitForTimeout(400);
  await expect(progress(page)).not.toContainText("Burst track");
  await call(page, "__releaseStickLoads");
});

test("safe to remove goes when the same Stick is seen mounted again, though nothing on it changed", async ({ page }) => {
  await ready(page);
  await pressFor(page, 2300);
  await view(page).getByTestId("stick-finished").getByRole("button", { name: "Eject DJ STICK" }).click();
  await expect(page.getByTestId("stick-safe-to-remove")).toHaveText(/DJ STICK is safe to remove/);
  // The same volume at the same place, its export unchanged, while the Library view shows.
  await tab(page, "Library").click();
  await page.evaluate((path) => {
    (window as unknown as { __remountDevice: (p: string) => void }).__remountDevice(path);
    window.dispatchEvent(new Event("focus"));
  }, DJ);
  await expect(page.getByTestId("shell-tree").getByRole("group", { name: "Stick DJ STICK" })).toBeVisible();
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-name")).toBeVisible();
  await expect(page.getByTestId("stick-safe-to-remove")).toHaveCount(0);
});
