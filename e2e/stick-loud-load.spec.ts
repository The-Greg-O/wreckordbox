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
  // The shared `open()` sometimes misses the first load after a fresh dev build (hold-to-load.md, not this ticket's).
  await open(page, "", prefs).catch(() => open(page, "", prefs));
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

test("with the setting off, the plain progress bar is shown; the switch on either swaps them and is remembered", async ({ page }) => {
  await ready(page, { loudStickLoad: false });
  await startLoad(page);
  await call(page, "__emitStickLoadTrack", DJ, 1, 4, ONE);
  await expect(progress(page)).toContainText(/Copying 1 of 4 · \S/);
  await expect(progress(page).getByRole("progressbar")).toHaveAttribute("aria-valuenow", "25");
  await expect(loud(page)).toHaveCount(0);
  await expect(progress(page).locator("canvas")).toHaveCount(0);

  // From the plain bar to the loud display, in place, the load going on.
  await progress(page).getByRole("button", { name: "Show the loud display" }).click();
  await expect(loud(page)).toHaveAttribute("data-track", ONE);
  await expect(loud(page)).toContainText("25%");
  // And back again, which is kept: a later load starts plain.
  await loud(page).getByRole("button", { name: "Show a plain bar" }).click();
  await expect(loud(page)).toHaveCount(0);
  await expect(progress(page).getByRole("progressbar")).toBeVisible();
  const kept = await page.evaluate(() => (JSON.parse(localStorage.getItem("rbl.preferences") ?? "{}") as { view?: { loudStickLoad?: boolean } }).view?.loudStickLoad);
  expect(kept).toBe(false);
  await call(page, "__releaseStickLoads");
  await expect(view(page).getByTestId("stick-finished")).toContainText("Loaded DJ STICK");
});

test("the setting is in Preferences › View › Layout, and turning it off there gives the next load the plain bar", async ({ page }) => {
  await ready(page);
  await page.keyboard.press("Shift+Control+P");
  const palette = page.getByRole("dialog", { name: /command/i });
  await palette.getByRole("combobox").fill("Preferences");
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Preferences" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("tab", { name: "View", exact: true }).click();
  await dialog.getByRole("tab", { name: "Layout", exact: true }).click();
  const box = dialog.getByRole("checkbox", { name: "Loud load display" });
  await expect(box).toBeChecked();
  await box.uncheck();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);

  await startLoad(page);
  await call(page, "__emitStickLoadTrack", DJ, 1, 4, ONE);
  await expect(progress(page)).toContainText(/Copying 1 of 4/);
  await expect(loud(page)).toHaveCount(0);
  await call(page, "__releaseStickLoads");
});

test("a track with no waveform, or one that cannot be read, is said, and never shows the track before it", async ({ page }) => {
  await ready(page);
  await startLoad(page);
  const canvas = loud(page).locator("canvas");
  await call(page, "__emitStickLoadTrack", DJ, 1, 4, ONE);
  await expect(loud(page)).toHaveAttribute("data-wave", "analysis");
  const real = await painted(canvas);

  await page.evaluate(() => { (window as unknown as { __failWaveformReads: (on: boolean) => void }).__failWaveformReads(true); });
  await call(page, "__emitStickLoadTrack", DJ, 2, 4, TWO);
  await expect(loud(page)).toHaveAttribute("data-wave", "failed");
  await expect(loud(page)).toContainText("Its waveform could not be read.");
  // Something is still drawn (a muted stand-in), and it is not the first track's waveform.
  const failed = await painted(canvas);
  expect(failed.inked).toBeGreaterThan(0.02);
  expect(failed.print).not.toBe(real.print);

  await page.evaluate(() => { (window as unknown as { __failWaveformReads: (on: boolean) => void }).__failWaveformReads(false); });
  // A track the service names by title only (two share it): no waveform to show, and it says so.
  await call(page, "__emitStickLoadTrack", DJ, 3, 4, null, "Shared title");
  await expect(loud(page).getByTestId("stick-loud-track")).toHaveText("Shared title");
  await expect(loud(page)).toHaveAttribute("data-wave", "none");
  await expect(loud(page)).toContainText("No waveform to show for this track.");
  await expect(loud(page)).not.toContainText("Its waveform could not be read.");
  await call(page, "__releaseStickLoads");
});

test("the loud display shrinks to the status line and comes back as the plain bar does, showing the track written meanwhile", async ({ page }) => {
  await ready(page);
  await startLoad(page);
  await call(page, "__emitStickLoadTrack", DJ, 1, 4, ONE);
  await expect(loud(page)).toHaveAttribute("data-track", ONE);

  await page.keyboard.press("Escape");
  await expect(loud(page)).toHaveCount(0);
  await expect(view(page).getByTestId("stick-shrunk")).toBeVisible();
  await expect(statusLoad(page)).toContainText("1/4");

  // Another view used meanwhile; the load goes on.
  await tab(page, "Library").click();
  await titles(page).first().click();
  await call(page, "__emitStickLoadTrack", DJ, 2, 4, TWO);
  await expect(statusLoad(page)).toContainText("2/4");

  await statusLoad(page).click();
  await expect(tab(page, "Stick")).toHaveAttribute("aria-selected", "true");
  await expect(loud(page)).toHaveAttribute("data-track", TWO);
  await expect(loud(page)).toHaveAttribute("data-wave", "analysis");
  await expect(loud(page)).toContainText("50%");
  expect((await painted(loud(page).locator("canvas"))).inked).toBeGreaterThan(0.05);

  // Its own shrink button does the same as Esc.
  await loud(page).getByRole("button", { name: /shrink to the status line/ }).click();
  await expect(loud(page)).toHaveCount(0);
  await statusLoad(page).click();
  await expect(loud(page)).toBeVisible();

  await call(page, "__releaseStickLoads");
  await expect(view(page).getByTestId("stick-finished")).toContainText("Loaded DJ STICK");
  await expect(statusLoad(page)).toHaveCount(0);
});

/** The median time between frames over `count` frames. */
async function medianFrame(page: Page, count = 180): Promise<number> {
  return page.evaluate(async (frames: number) => {
    const deltas: number[] = [];
    let last = performance.now();
    await new Promise<void>((done) => {
      const tick = () => {
        const now = performance.now();
        deltas.push(now - last);
        last = now;
        if (deltas.length < frames) requestAnimationFrame(tick);
        else done();
      };
      requestAnimationFrame(tick);
    });
    deltas.sort((a, b) => a - b);
    return deltas[deltas.length >> 1] ?? 0;
  }, count);
}

/**
 * Feeds the load a track every 50 ms (the two tracks in turn, each a new
 * waveform to read and draw) for `ms`, and answers how late the feed ran:
 * the window's share of what could slow a load is how promptly it takes in
 * the load's progress.
 */
async function feed(page: Page, ms: number): Promise<number> {
  return page.evaluate(async ([path, one, two, span]) => {
    const emit = (window as unknown as Record<string, (...a: unknown[]) => void>).__emitStickLoadTrack!;
    const start = performance.now();
    let late = 0;
    let at = 0;
    for (let step = 1; at < span; step += 1) {
      const due = start + step * 50;
      await new Promise((go) => setTimeout(go, Math.max(0, due - performance.now())));
      at = performance.now() - start;
      late += Math.max(0, performance.now() - due);
      emit(path, step, 1000, step % 2 === 0 ? one : two);
    }
    return late;
  }, [DJ, ONE, TWO, ms] as const);
}

test("the display costs no more per frame than the plain bar, within the deck's budget, and takes in progress as promptly", async ({ page }) => {
  test.setTimeout(120_000);
  const runs: Record<"plain" | "loud", { frame: number; late: number }[]> = { plain: [], loud: [] };
  await ready(page, { loudStickLoad: false });
  await startLoad(page);
  await call(page, "__emitStickLoadTrack", DJ, 1, 1000, ONE);
  // The same load, plain and loud in turn (switched in place), twice: a
  // headless browser's timers are noisy, so the best of each is compared.
  for (const kind of ["plain", "loud", "plain", "loud"] as const) {
    if (kind === "loud") {
      await progress(page).getByRole("button", { name: "Show the loud display" }).click();
      await expect(loud(page)).toHaveAttribute("data-wave", "analysis");
    } else {
      if (await loud(page).count() > 0) await loud(page).getByRole("button", { name: "Show a plain bar" }).click();
      await expect(loud(page)).toHaveCount(0);
    }
    // While tracks keep coming: frames measured, and how late the feed ran.
    const [frame, late] = await Promise.all([medianFrame(page), feed(page, 3000)]);
    runs[kind].push({ frame, late });
  }
  const best = (kind: "plain" | "loud", of: "frame" | "late") => Math.min(...runs[kind].map((run) => run[of]));
  await expect(loud(page)).toBeVisible();
  await call(page, "__releaseStickLoads");
  await expect(view(page).getByTestId("stick-finished")).toBeVisible();
  test.info().annotations.push({ type: "cost", description: JSON.stringify(runs) });
  console.log(`cost ${JSON.stringify(runs)}`);
  expect(best("loud", "frame") - best("plain", "frame")).toBeLessThan(budgets.gates.browser.deckFrameCostMs.chromium);
  // Sixty updates: lateness over the whole feed within 2 ms an update of the plain bar's.
  expect(best("loud", "late") - best("plain", "late")).toBeLessThan(120);
});

const SHOTS = process.env.LOUD_SHOTS;

for (const width of [1280, 900, 640, 436]) {
  test(`at ${width} px the display stays within the view, its parts apart, with no sideways scrolling`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await ready(page);
    await startLoad(page);
    await call(page, "__emitStickLoadTrack", DJ, 37, 64, TWO);
    await expect(loud(page)).toHaveAttribute("data-wave", "analysis");
    await expect(loud(page).getByTestId("stick-loud-detail")).toContainText("BPM");

    expect(await view(page).evaluate((el) => el.scrollWidth <= el.clientWidth + 1)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(true);
    const parts = await loud(page).evaluate((stage) => {
      const bounds = stage.getBoundingClientRect();
      const inside = [...stage.querySelectorAll("h2, p, button, [role=progressbar]")].map((el) => {
        const r = el.getBoundingClientRect();
        return { what: el.textContent?.slice(0, 30) ?? "", left: r.left - bounds.left, right: bounds.right - r.right, top: r.top - bounds.top, bottom: bounds.bottom - r.bottom };
      });
      const box = (sel: string) => stage.querySelector(sel)!.getBoundingClientRect();
      const [percent, track] = [box("p[aria-hidden]"), box("[data-testid=stick-loud-track]")];
      const overlap = !(percent.right <= track.left || track.right <= percent.left || percent.bottom <= track.top || track.bottom <= percent.top);
      return { inside, overlap };
    });
    for (const part of parts.inside) {
      for (const side of ["left", "right", "top", "bottom"] as const) expect(part[side], `${part.what}: ${side}`).toBeGreaterThanOrEqual(-0.5);
    }
    expect(parts.overlap, "the percentage and the track's title overlap").toBe(false);

    if (SHOTS) {
      const { LATTE, TOKYO_NIGHT, WHITE, announce } = await import("./lightPalettes");
      for (const [name, roles] of [["dark", TOKYO_NIGHT], ["latte", LATTE], ["white", WHITE]] as const) {
        await announce(page, { roles, font: null });
        await page.waitForTimeout(200);
        await page.screenshot({ path: `${SHOTS}/loading-${name}-${width}.png`, animations: "disabled" });
      }
    }
    await call(page, "__releaseStickLoads");
  });
}
