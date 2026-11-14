/**
 * The loud load display (wbx-73): while a Stick load runs, the Stick view
 * fills with a loud, moving display drawn from the waveform of the track
 * being written, its title, artist and tempo, and a very large percentage.
 * A setting (`view.loudStickLoad`, also switched from the display itself and
 * in Preferences) swaps it for wbx-72's plain bar. It shrinks to the status
 * line and comes back exactly as the plain bar does, and it costs the window
 * nothing measurable per frame or in how fast progress is taken in.
 *
 * Mock hooks: `src/ipc/mock-stick-load.ts` (`__emitStickLoadTrack` names a
 * Library track, whose waveform and details are the mock's own).
 */
import { expect, test, type Locator, type Page } from "@playwright/test";

import budgets from "../perf-budgets.json" with { type: "json" };
import { open, titles } from "./compactPlayer";

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const view = (page: Page) => page.getByTestId("stick-view");
const holdButton = (page: Page) => view(page).getByTestId("stick-load");
const progress = (page: Page) => view(page).getByTestId("stick-progress");
const loud = (page: Page) => view(page).getByTestId("stick-loud");
const statusLoad = (page: Page) => page.getByTestId("status-stick-load");

const DJ = "/Volumes/DJ STICK";
/** Two analysed mock tracks (rows 3 and 4 of the mock Library). */
const ONE = "100003";
const TWO = "100004";

async function call<T>(page: Page, name: string, ...args: unknown[]): Promise<T> {
  return page.evaluate(([hook, rest]) => {
    const hooks = window as unknown as Record<string, (...a: unknown[]) => T>;
    return hooks[hook]!(...rest);
  }, [name, args] as [string, unknown[]]);
}

/** The Stick view, DJ STICK unlocked with its first playlist chosen; `prefs` seeds the view preferences. */
async function ready(page: Page, prefs: Record<string, unknown> = {}): Promise<void> {
  await open(page, "", prefs);
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-name")).toHaveText("DJ STICK");
  await view(page).getByTestId("stick-choose").locator("label").filter({ hasNotText: "(folder" }).first().getByRole("checkbox").check();
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  const field = view(page).getByRole("textbox", { name: "Type DJ STICK to unlock it" });
  if (await field.count() > 0) await field.fill("DJ STICK");
  await view(page).getByRole("button", { name: "Unlock DJ STICK" }).click();
  await expect(holdButton(page)).toBeEnabled();
}

/** Starts a held load (it runs until `__releaseStickLoads`). */
async function startLoad(page: Page): Promise<void> {
  await call(page, "__holdStickLoads");
  await holdButton(page).hover();
  await page.mouse.down();
  await page.waitForTimeout(2300);
  await page.mouse.up();
  await expect(progress(page)).toBeVisible();
}

/** What the display's canvas has painted: the share of pixels with ink, and a fingerprint of them. */
function painted(canvas: Locator): Promise<{ inked: number; print: number }> {
  return canvas.evaluate((el) => {
    const c = el as HTMLCanvasElement;
    const data = c.getContext("2d")!.getImageData(0, 0, c.width, c.height).data;
    let inked = 0;
    let print = 0;
    for (let i = 0; i < data.length; i += 4) {
      if (data[i + 3]! > 0) inked += 1;
      print = (print * 31 + data[i]! + data[i + 1]! * 3 + data[i + 2]! * 7 + data[i + 3]! * 11) % 1_000_000_007;
    }
    return { inked: inked / (data.length / 4), print };
  });
}

test("with the setting on, a running load fills the view with the display, drawn from the track being written", async ({ page }) => {
  await ready(page);
  await startLoad(page);
  await expect(loud(page)).toBeVisible();
  // It fills the view.
  const [box, whole] = [await loud(page).boundingBox(), await view(page).boundingBox()];
  expect(box!.width * box!.height).toBeGreaterThan(0.9 * whole!.width * whole!.height);

  await call(page, "__emitStickLoadTrack", DJ, 1, 4, ONE);
  await expect(loud(page)).toHaveAttribute("data-track", ONE);
  const canvas = loud(page).locator("canvas");
  await expect(loud(page)).toHaveAttribute("data-wave", "analysis");
  const first = await painted(canvas);
  expect(first.inked).toBeGreaterThan(0.05);
  await expect(loud(page)).toContainText("25%");
  await expect(loud(page).getByTestId("stick-loud-track")).toContainText(/\S/);
  const firstTitle = await loud(page).getByTestId("stick-loud-track").textContent();

  // The next track: its own waveform, title and percentage.
  await call(page, "__emitStickLoadTrack", DJ, 2, 4, TWO);
  await expect(loud(page)).toHaveAttribute("data-track", TWO);
  await expect(loud(page)).toContainText("50%");
  await expect(loud(page).getByTestId("stick-loud-track")).not.toHaveText(firstTitle!);
  await expect.poll(async () => (await painted(canvas)).print).not.toBe(first.print);
  // The title, artist and tempo come from the Library's details of that track.
  await expect(loud(page).getByTestId("stick-loud-detail")).toContainText(/\d{2,3}(\.\d+)? BPM/);

  await call(page, "__releaseStickLoads");
  await expect(loud(page)).toHaveCount(0);
  await expect(view(page).getByTestId("stick-finished")).toContainText("Loaded DJ STICK");
});
