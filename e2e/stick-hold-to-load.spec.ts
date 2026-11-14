/**
 * Hold to load (wbx-72): a load starts only when the DJ holds the Stick
 * view's big button (or `L`) for the whole hold, longer for a Gig stick
 * than a Spare stick; a short press or an early release writes nothing.
 * While it runs it shows a plain progress bar naming the track being
 * written; `Esc` shrinks it to the status line, where it keeps running
 * while other views are used, and a click there brings it back. It ends in
 * "safe to remove" with a short summary.
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

test("a full hold starts the load: a plain bar names the track being written, and it ends safe to remove with a summary", async ({ page }) => {
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
  await expect(done).toContainText("DJ STICK is safe to remove");
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
  await expect(view(page).getByTestId("stick-finished")).toContainText("TEST is safe to remove");
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
  await expect(view(page).getByTestId("stick-finished")).toContainText("DJ STICK is safe to remove");
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
  await expect(view(page).getByTestId("stick-finished")).toContainText("DJ STICK is safe to remove");
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
  await expect(view(page).getByTestId("stick-finished")).toContainText("DJ STICK is safe to remove");
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
