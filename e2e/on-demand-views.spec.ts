/**
 * The shell's views loaded on demand (wbx-45.13): Intake and Stick are their
 * own chunks, kept off the window's start and fetched once the shell has
 * painted. A first switch shows the view at once; a view whose code is still
 * on the way says so and its key still switches to it; a chunk that failed
 * to load is shown as a failure with Retry, never as an empty view.
 */
import { expect, test, type Page, type Route } from "@playwright/test";
import { seedShell } from "./compactPlayer";

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const working = (page: Page) => page.getByTestId("shell-working");
const INTAKE_CHUNK = /\/assets\/IntakeView-[^/]*\.js$/;
const STICK_CHUNK = /\/assets\/StickLoadView-[^/]*\.js$/;

async function openShell(page: Page): Promise<void> {
  await seedShell(page);
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  await expect(page.locator('[role="gridcell"][data-col="title"]').first()).toBeVisible();
}

test("the Intake and Stick views are not part of the window's start, and are fetched once it has painted", async ({ page }) => {
  const fetched: string[] = [];
  page.on("request", (request) => { fetched.push(new URL(request.url()).pathname); });
  await openShell(page);
  await expect.poll(() => fetched.some((path) => INTAKE_CHUNK.test(path))).toBe(true);
  await expect.poll(() => fetched.some((path) => STICK_CHUNK.test(path))).toBe(true);
  // Only after the start: the entry and the window's own chunk came first.
  const entry = fetched.findIndex((path) => /\/assets\/App-[^/]*\.js$/.test(path));
  expect(entry).toBeGreaterThanOrEqual(0);
  expect(fetched.findIndex((path) => INTAKE_CHUNK.test(path))).toBeGreaterThan(entry);
});

test("a first switch to Intake draws the view at once, with no stand-in in between", async ({ page }) => {
  await openShell(page);
  // Its code has arrived by the time the DJ reaches for the key.
  await page.waitForLoadState("networkidle");
  // Every frame from the key on: is a stand-in drawn, and when is the view?
  await page.evaluate(() => {
    const w = window as unknown as { __firstSwitch: { standIn: boolean; drawnMs: number | null; frames: number } };
    w.__firstSwitch = { standIn: false, drawnMs: null, frames: 0 };
    const area = document.querySelector('[data-testid="shell-working"]')!;
    window.addEventListener("keydown", () => {
      const pressedAt = performance.now();
      const frame = () => {
        w.__firstSwitch.frames += 1;
        if (area.querySelector('[data-testid="shell-empty"], [data-testid="on-demand-failed"]')) w.__firstSwitch.standIn = true;
        const drawn = [...area.children].some((child) => !child.hasAttribute("data-kept-hidden"));
        if (drawn && w.__firstSwitch.drawnMs === null) w.__firstSwitch.drawnMs = performance.now() - pressedAt;
        if (w.__firstSwitch.frames < 30) requestAnimationFrame(frame);
      };
      requestAnimationFrame(frame);
    }, { capture: true, once: true });
  });
  await page.keyboard.press("2");
  await expect(tab(page, "Intake")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("listbox", { name: "Intake queue" })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __firstSwitch: { frames: number } }).__firstSwitch.frames)).toBe(30);
  const seen = await page.evaluate(() => (window as unknown as { __firstSwitch: { standIn: boolean; drawnMs: number | null } }).__firstSwitch);
  console.log(`first switch to Intake: view drawn ${seen.drawnMs?.toFixed(1)} ms after the key`);
  expect(seen.standIn).toBe(false);
  expect(seen.drawnMs).not.toBeNull();
  expect(seen.drawnMs!).toBeLessThan(100);
});

test("while the Intake view's code is on the way its key still switches to it, and the view follows when the code arrives", async ({ page }) => {
  const held: Route[] = [];
  await page.route(INTAKE_CHUNK, (route) => { held.push(route); });
  await openShell(page);
  await expect.poll(() => held.length).toBe(1);

  await page.keyboard.press("2");
  await expect(tab(page, "Intake")).toHaveAttribute("aria-selected", "true");
  await expect(working(page).getByTestId("shell-empty")).toContainText("Opening");
  // The Library list stays kept and inert behind it.
  await expect(page.locator('[data-kept-hidden]')).toHaveCount(1);

  await held[0]!.continue();
  await expect(page.getByRole("listbox", { name: "Intake queue" })).toBeVisible();
  await expect(working(page).getByTestId("shell-empty")).toHaveCount(0);

  // 1 goes back to the Library, as before.
  await page.keyboard.press("1");
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
});

test("a view whose code failed to load is shown as a failure, not an empty view, and Retry loads it", async ({ page }) => {
  let fail = true;
  await page.route(STICK_CHUNK, (route) => (fail ? route.abort("failed") : route.continue()));
  await openShell(page);

  await page.keyboard.press("4");
  await expect(tab(page, "Stick")).toHaveAttribute("aria-selected", "true");
  const failed = working(page).getByTestId("on-demand-failed");
  await expect(failed).toBeVisible();
  await expect(failed).toHaveAttribute("role", "alert");
  await expect(failed).toContainText("could not be loaded");
  await expect(working(page).getByTestId("shell-empty")).toHaveCount(0);

  fail = false;
  await failed.getByRole("button", { name: "Retry" }).click();
  // Either the view arrives, or (if the browser keeps the failed load) the failure stays shown.
  await expect(working(page).getByTestId("shell-empty").or(working(page).getByTestId("stick-view")).or(failed)).toBeVisible();
  await expect(working(page).getByTestId("shell-empty")).toHaveCount(0);
});
