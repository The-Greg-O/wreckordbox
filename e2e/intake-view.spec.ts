/**
 * The Intake view (wbx-62): the Intake queue (every track with no Prep
 * colour) with what each track still needs and the count on the view's tab;
 * selecting a queued track loads it into the large player (detail waveform
 * with beat grid and phrases, and the overview); an empty queue says so and
 * offers Problems, which is the same view over tracks marked Problem.
 *
 * Mock backend: Intake is the mock's uncoloured rows; Problems is its Red
 * ones (`src/ipc/mock-intake.ts`). `?intake=empty` colours every row;
 * `?noProblemColour` names no colour Problem.
 */
import { expect, test, type Page } from "@playwright/test";
import { engine, player, seedShell } from "./compactPlayer";

const tabs = (page: Page) => page.getByRole("tablist", { name: "Views" });
const tab = (page: Page, name: string) => tabs(page).getByRole("tab", { name });
const tabCount = (page: Page) => page.getByTestId("shell-tab-count-intake");
const tree = (page: Page) => page.getByRole("tree", { name: "Tree" });
const treeItem = (page: Page, name: string) => tree(page).getByRole("group", { name: "Library" }).getByRole("treeitem", { name });
const queue = (page: Page, name = "Intake queue") => page.getByRole("listbox", { name });
const options = (page: Page, name?: string) => queue(page, name).getByRole("option");
const loadedTitle = (page: Page) => player(page).getByTestId("compact-player-title");
const detail = (page: Page) => player(page).getByTestId("large-player-detail");

async function openIntake(page: Page, query = ""): Promise<void> {
  await seedShell(page);
  await page.goto(`/${query}`);
  await expect(page.getByTestId("shell")).toBeVisible();
  await tab(page, "Intake").click();
  await expect(tab(page, "Intake")).toHaveAttribute("aria-selected", "true");
}

test("the Intake view lists the queue with what each track still needs, and the tab and tree show how many wait", async ({ page }) => {
  await openIntake(page);

  await expect(options(page).first()).toBeVisible();
  const waiting = Number(await tabCount(page).innerText());
  expect(waiting).toBeGreaterThan(0);
  await expect(treeItem(page, "Intake")).toContainText(String(waiting));
  await expect(page.getByTestId("intake-position")).toHaveText(String(waiting));

  // Each row: title, then artist · BPM · key, then what it still needs.
  const first = options(page).first();
  await expect(first.getByTestId("intake-needs")).toHaveText(/^(needs .+|nothing missing)$/);
  // Rows are three lines each; at least one shows a specific need.
  const lines = await options(page).getByTestId("intake-needs").allInnerTexts();
  expect(lines.some((line) => /needs (everything|.*(Energy|cues|grid|tags|Situation|Components))/.test(line))).toBe(true);
});

test("the Intake tab's count follows an edit as it happens", async ({ page }) => {
  await openIntake(page);
  await expect(options(page).first()).toBeVisible();
  const before = Number(await tabCount(page).innerText());
  const firstId = await options(page).first().getAttribute("data-track");
  const firstTitle = await options(page).first().locator("span").first().innerText();

  // Coloured Clean elsewhere (the Library list, the CLI): it leaves the queue.
  await page.evaluate((id) => (window as unknown as { __setPrepColour: (id: string, c: number) => Promise<unknown> }).__setPrepColour(id!, 5), firstId);

  await expect(tabCount(page)).toHaveText(String(before - 1));
  await expect(treeItem(page, "Intake")).toContainText(String(before - 1));
  await expect(queue(page).locator(`[data-track="${firstId}"]`)).toHaveCount(0);
  await expect(options(page).first()).not.toHaveText(new RegExp(`^${firstTitle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`));

  // And back: taking the colour off puts it in the queue again.
  await page.evaluate((id) => (window as unknown as { __setPrepColour: (id: string, c: number) => Promise<unknown> }).__setPrepColour(id!, 0), firstId);
  await expect(tabCount(page)).toHaveText(String(before));
  await expect(queue(page).locator(`[data-track="${firstId}"]`)).toHaveCount(1);
});

test("selecting a queued track loads it into the large player: detail waveform, beat grid, phrases and overview", async ({ page }) => {
  await openIntake(page);
  // A row with a tempo (an analysed track: it has a grid and phrases).
  const analysed = options(page).filter({ hasText: /· \d{2,3} ·/ }).first();
  const title = await analysed.locator("span").first().innerText();
  await analysed.click();

  await expect(analysed).toHaveAttribute("aria-selected", "true");
  await expect(loadedTitle(page)).toHaveText(title);
  await expect(page.getByTestId("intake-position")).toHaveText(/^\d+ of \d+$/);
  await expect(page.getByRole("region", { name: "Track" }).getByRole("textbox", { name: "Title" })).toHaveValue(title);

  await expect(detail(page)).toBeVisible();
  await expect(detail(page).getByTestId("beat-grid-marker").first()).toBeAttached();
  expect(await detail(page).getByTestId("beat-grid-marker").count()).toBeGreaterThan(8);
  await expect(player(page).getByTestId("large-player-phrases").locator("[data-phrase]").first()).toBeVisible();
  await expect(player(page).getByTestId("compact-player-overview")).toBeVisible();
  // Phrases run above the detail waveform and under the overview.
  const phrases = await player(page).getByTestId("large-player-phrases").boundingBox();
  const wave = await player(page).getByTestId("large-player-wave").boundingBox();
  const over = await player(page).getByTestId("compact-player-overview").boundingBox();
  expect(phrases!.y).toBeGreaterThan(over!.y);
  expect(phrases!.y + phrases!.height).toBeLessThanOrEqual(wave!.y + 1);
  // The detail is drawn at device resolution, like every canvas.
  const canvas = player(page).getByTestId("large-player-wave").locator("canvas").first();
  await expect(canvas).toBeAttached();
});

test("the large player is Intake's: the other views keep the compact player, and the track plays on across the switch", async ({ page }) => {
  await openIntake(page);
  const row = options(page).filter({ hasText: /· \d{2,3} ·/ }).first();
  const title = await row.locator("span").first().innerText();
  await row.click();
  await expect(detail(page)).toBeVisible();
  await player(page).getByRole("button", { name: "Play", exact: true }).click();
  await expect(player(page).getByRole("button", { name: "Pause", exact: true })).toBeVisible();

  await tab(page, "Library").click();
  await expect(detail(page)).toHaveCount(0);
  await expect(loadedTitle(page)).toHaveText(title);
  await expect(player(page).getByRole("button", { name: "Pause", exact: true })).toBeVisible();

  await tab(page, "Intake").click();
  await expect(detail(page)).toBeVisible();
  // Coming back finds the loaded track selected in the queue.
  await expect(queue(page).getByRole("option", { selected: true })).toContainText(title);
});

test("the queue's keys move the selection and load the track, and stop there", async ({ page }) => {
  await openIntake(page);
  await options(page).first().click();
  const second = await options(page).nth(1).locator("span").first().innerText();
  const treeSelected = await tree(page).getByRole("treeitem", { selected: true }).getAttribute("data-row-id");

  await queue(page).press("ArrowDown");
  await expect(options(page).nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(loadedTitle(page)).toHaveText(second);
  // The tree's selection did not move, and Space did not start playback.
  await expect(tree(page).getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", treeSelected!);
  await queue(page).press(" ");
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();
  // Up comes back; the deck's arrows (beat jumps) did not hear any of it.
  await queue(page).press("ArrowUp");
  await expect(options(page).first()).toHaveAttribute("aria-selected", "true");
  // A fresh load is at the start; a beat jump would have moved it seconds on.
  expect(Math.abs((await engine(page)).a)).toBeLessThan(1);
  // 1 to 4 still switch views from the queue.
  await queue(page).press("1");
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
});

test("an empty Intake queue says so and offers Problems, which is the same view", async ({ page }) => {
  await openIntake(page, "?intake=empty");

  await expect(page.getByTestId("intake-empty")).toContainText("Intake is empty");
  await expect(tabCount(page)).toHaveText("0");
  await page.getByRole("button", { name: /Show Problems/ }).click();

  await expect(page.getByTestId("intake-view")).toHaveAttribute("data-queue", "problems");
  await expect(options(page, "Problems queue").first()).toBeVisible();
  await expect(treeItem(page, "Problems")).toHaveAttribute("aria-selected", "true");
  const title = await options(page, "Problems queue").first().locator("span").first().innerText();
  await options(page, "Problems queue").first().click();
  await expect(loadedTitle(page)).toHaveText(title);
  await expect(detail(page)).toBeVisible();
});

test("Problems from the tree uses the same view; with no colour named Problem it says why", async ({ page }) => {
  await openIntake(page);
  await treeItem(page, "Problems").click();
  await expect(page.getByTestId("intake-view")).toHaveAttribute("data-queue", "problems");
  await expect(options(page, "Problems queue").first()).toBeVisible();
  await expect(options(page, "Problems queue").first().getByTestId("intake-needs")).toBeVisible();

  await openIntake(page, "?noProblemColour");
  await treeItem(page, "Problems").click();
  await expect(page.getByTestId("intake-empty")).toContainText("No colour label is named Problem");
  await page.getByRole("button", { name: "Show Intake" }).click();
  await expect(page.getByTestId("intake-view")).toHaveAttribute("data-queue", "intake");
});

test("the queue never pushes the large player out of a half-width window", async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 800 });
  await openIntake(page);
  // At half width the queue is a counter (wbx-69; intake-half-width.spec.ts).
  const next = page.getByRole("button", { name: "Next track", exact: true });
  await next.click();
  await next.click();
  await expect(page.getByTestId("intake-position")).toHaveText(/^2 of \d+$/);
  await expect(detail(page)).toBeVisible();
  const box = await detail(page).boundingBox();
  expect(box!.width).toBeLessThanOrEqual(640);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(640);
});


// ---- Review round (codex/review-wbx-62.md) ----

type Hooks = {
  __libraryReady: () => void;
  __setFileMissing: (id: string, missing: boolean) => void;
  __intakeReads: () => { counts: number; queue: number };
  __holdIntakeReads: () => void;
  __releaseIntakeReads: () => void;
  __failIntakeReads: (on: boolean) => void;
  __emitLibraryChanged: (times: number) => void;
  __setPrepColour: (id: string, colour: number) => Promise<unknown>;
};
/** Calls one of the mock's test hooks on `window`. */
const call = <R = unknown>(page: Page, name: keyof Hooks, ...args: unknown[]): Promise<R> =>
  page.evaluate(([hook, values]) => {
    const fn = (window as unknown as Record<string, (...given: unknown[]) => unknown>)[hook];
    if (!fn) throw new Error(`no mock hook ${hook}`);
    return fn(...values);
  }, [name, args] as [string, unknown[]]) as Promise<R>;
const playing = (page: Page) => player(page).getByRole("button", { name: "Pause", exact: true });
const playButton = (page: Page) => player(page).getByRole("button", { name: "Play", exact: true });
const analysedRow = (page: Page, n = 0) => options(page).filter({ hasText: /· \d{2,3} ·/ }).nth(n);
const rowTitle = async (row: ReturnType<typeof analysedRow>) => row.locator("span").first().innerText();

test("a Library still loading at start shows the counts and the queue once it is ready, with no edit", async ({ page }) => {
  await openIntake(page, "?slow");
  await expect(tabCount(page)).toHaveCount(0);
  await expect(page.getByTestId("intake-loading")).toBeVisible();

  await call(page, "__libraryReady");

  await expect(tabCount(page)).toHaveText(/^\d+$/);
  await expect(options(page).first()).toBeVisible();
  await expect(treeItem(page, "Intake")).toContainText(await tabCount(page).innerText());
});

for (const state of ["paused", "playing"] as const) {
  test(`the detail waveform and its playhead are in place after Intake, Library and Intake again (${state})`, async ({ page }) => {
    await openIntake(page);
    await analysedRow(page).click();
    await expect(detail(page).locator("canvas").first()).toBeAttached();
    // Somewhere in the middle of the track, so the layer has somewhere to be.
    const over = player(page).getByTestId("compact-player-overview");
    const box = (await over.boundingBox())!;
    await page.mouse.click(box.x + box.width * 0.4, box.y + box.height / 2);
    if (state === "playing") {
      await playButton(page).click();
      await expect(playing(page)).toBeVisible();
    }
    const placed = async () => detail(page).evaluate((band) => {
      const head = band.querySelector<HTMLElement>('[data-testid="large-player-head"]')!;
      const layer = head.parentElement!.firstElementChild as HTMLElement;
      const b = band.getBoundingClientRect();
      const h = head.getBoundingClientRect();
      return { headOffset: Math.round(h.left + h.width / 2 - (b.left + b.width / 2)), head: head.style.transform, layer: layer.style.transform };
    });
    const before = await placed();
    expect(before.head).toMatch(/translateX\(\d/);
    expect(Math.abs(before.headOffset)).toBeLessThanOrEqual(2);

    await tab(page, "Library").click();
    await expect(detail(page)).toHaveCount(0);
    await tab(page, "Intake").click();
    await expect(detail(page)).toBeVisible();

    await expect.poll(async () => (await placed()).head).toMatch(/translateX\(\d/);
    const after = await placed();
    expect(Math.abs(after.headOffset)).toBeLessThanOrEqual(2);
    expect(after.layer).toMatch(/translateX\(/);
    if (state === "paused") expect(after.layer).toBe(before.layer);
    else await expect.poll(async () => (await placed()).layer).not.toBe(after.layer);
  });
}

for (const key of ["Enter", " "] as const) {
  test(`the next page of the queue is reached by keyboard (${key === " " ? "Space" : key}), without loading a track`, async ({ page }) => {
    await openIntake(page);
    await expect(options(page)).toHaveCount(200);
    const firstTitle = await rowTitle(options(page).first());
    const next = page.getByRole("button", { name: /Next/ });
    await next.focus();
    await page.keyboard.press(key);

    await expect(options(page).first()).not.toHaveText(new RegExp(`^${firstTitle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`));
    await expect(page.getByTestId("intake-page")).toContainText("201");
    await expect(loadedTitle(page)).toHaveText("");
  });
}

test("the empty state's buttons are pressed by Space without playing or pausing the deck", async ({ page }) => {
  await seedShell(page);
  await page.goto("/?intake=empty");
  await page.locator('[role="gridcell"][data-col="title"]').nth(3).dblclick();
  await playButton(page).click();
  await expect(playing(page)).toBeVisible();
  await tab(page, "Intake").click();

  await page.getByRole("button", { name: /Show Problems/ }).focus();
  await page.keyboard.press(" ");
  await expect(page.getByTestId("intake-view")).toHaveAttribute("data-queue", "problems");
  await expect(playing(page)).toBeVisible();
});

test("selecting a queued track replaces the loaded one without asking, even while it plays", async ({ page }) => {
  await openIntake(page);
  await analysedRow(page, 0).click();
  await playButton(page).click();
  await expect(playing(page)).toBeVisible();
  const next = await rowTitle(analysedRow(page, 1));

  await analysedRow(page, 1).click();

  await expect(loadedTitle(page)).toHaveText(next);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(analysedRow(page, 1)).toHaveAttribute("aria-selected", "true");
});

test("a track that cannot load leaves the queue on the track that did, and says why", async ({ page }) => {
  await openIntake(page);
  const loaded = analysedRow(page, 0);
  const title = await rowTitle(loaded);
  await loaded.click();
  await expect(loadedTitle(page)).toHaveText(title);
  const broken = analysedRow(page, 1);
  const brokenId = await broken.getAttribute("data-track");
  await call(page, "__setFileMissing", brokenId, true);

  await broken.click();

  await expect(page.getByTestId("intake-load-failed")).toContainText(await rowTitle(broken));
  await expect(loadedTitle(page)).toHaveText(title);
  await expect(loaded).toHaveAttribute("aria-selected", "true");
  await expect(broken).toHaveAttribute("aria-selected", "false");
  await expect(page.getByRole("region", { name: "Track" }).getByRole("textbox", { name: "Title" })).toHaveValue(title);
  await expect(page.getByTestId("shell-status")).toContainText(/could not be found/i);
});

test("a burst of a hundred Library changes while a read is held costs at most one more read, and ends fresh", async ({ page }) => {
  await openIntake(page);
  await expect(options(page).first()).toBeVisible();
  const before = Number(await tabCount(page).innerText());
  const id = await options(page).first().getAttribute("data-track");
  const start = await call<{ counts: number; queue: number }>(page, "__intakeReads");

  await call(page, "__holdIntakeReads");
  await call(page, "__emitLibraryChanged", 1);
  // A real change during the burst, so the read after it must see it.
  await call(page, "__setPrepColour", id, 5);
  await call(page, "__emitLibraryChanged", 100);
  await call(page, "__releaseIntakeReads");

  await expect(tabCount(page)).toHaveText(String(before - 1));
  await expect(queue(page).locator(`[data-track="${id}"]`)).toHaveCount(0);
  const end = await call<{ counts: number; queue: number }>(page, "__intakeReads");
  // The held read, and one more for the whole burst, for each reader.
  expect(end.counts - start.counts).toBeLessThanOrEqual(2);
  expect(end.queue - start.queue).toBeLessThanOrEqual(2);
});

test("a refresh that fails keeps what was read, marked stale, with a retry", async ({ page }) => {
  await openIntake(page);
  await expect(options(page).first()).toBeVisible();
  const rows = await options(page).count();

  await call(page, "__failIntakeReads", true);
  await call(page, "__emitLibraryChanged", 1);

  await expect(page.getByTestId("intake-stale")).toContainText(/could not be refreshed/i);
  await expect(page.getByTestId("intake-view")).toHaveAttribute("data-stale", "");
  await expect(options(page)).toHaveCount(rows);
  await expect(page.getByTestId("shell-status")).toContainText(/Intake/);

  await call(page, "__failIntakeReads", false);
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByTestId("intake-stale")).toHaveCount(0);
  await expect(page.getByTestId("intake-view")).not.toHaveAttribute("data-stale", "");
});

test("the detail waveform is drawn, backed at the pixels it covers", async ({ page }) => {
  await openIntake(page);
  await analysedRow(page).click();
  const canvas = player(page).getByTestId("large-player-wave").locator("canvas").first();
  await expect.poll(async () => canvas.evaluate((element: HTMLCanvasElement) => {
    const zoom = Number(document.documentElement.style.getPropertyValue("zoom") || 1);
    const ratio = window.devicePixelRatio * zoom;
    const want = { w: Math.round(element.clientWidth * ratio), h: Math.round(element.clientHeight * ratio) };
    if (Math.abs(element.width - want.w) > 1 || Math.abs(element.height - want.h) > 1) return `${element.width}x${element.height} for ${want.w}x${want.h}`;
    const data = element.getContext("2d")!.getImageData(0, 0, element.width, element.height).data;
    let lit = 0;
    for (let i = 3; i < data.length; i += 4 * 16) if ((data[i] ?? 0) > 0) lit += 1;
    return lit > 100 ? "ok" : `only ${lit} sampled pixels drawn`;
  })).toBe("ok");
});
