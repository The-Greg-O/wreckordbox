/**
 * Splitting Stems from Intake (wbx-67): the large player's Stems control
 * starts a separation of the loaded track. It shows as a Run in the status
 * line with its progress and blocks nothing; when it is done the track
 * shows it has Stems. Current Stems are split again only after the DJ
 * confirms; stale ones are labelled; a failure is shown with its reason.
 * A split belongs to the shell, so it survives switching views and
 * tracks, and a track is never split twice at once.
 *
 * The mock's separation is fake (`src/ipc/mock-stems.ts`): progress every
 * 50 ms, a Run while it works, hooks to hold it, fail it, burst its
 * progress and count how often the "engine" was asked.
 */
import { expect, test, type Page } from "@playwright/test";
import { player, seedShell } from "./compactPlayer";
import { audit, shortfalls } from "./lightAudit";
import { LATTE, WHITE, announce } from "./lightPalettes";

type Hooks = Record<string, (...args: unknown[]) => unknown>;

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const queue = (page: Page) => page.getByRole("listbox", { name: "Intake queue" });
const options = (page: Page) => queue(page).getByRole("option");
const panel = (page: Page) => page.getByRole("region", { name: "Track" });
const stems = (page: Page) => player(page).getByRole("group", { name: "Stems" });
const stemsState = (page: Page) => stems(page).getByTestId("stems-state");
const splitButton = (page: Page) => stems(page).getByRole("button", { name: /^(Split|Split again…|Retry)$/ });
const statusLine = (page: Page) => page.getByTestId("status-line");
const loadedTitle = (page: Page) => player(page).getByTestId("compact-player-title");

const call = (page: Page, name: string, ...args: unknown[]) =>
  page.evaluate(([hook, rest]) => (window as unknown as Hooks)[hook]!(...(rest)), [name, args] as const);
const runs = async (page: Page, id?: string) => (await call(page, "__stemsRuns", id)) as number;

async function openIntake(page: Page, query = "?writable=1&stemsDelay=1200"): Promise<void> {
  await seedShell(page);
  await page.goto(`/${query}`);
  await expect(page.getByTestId("shell")).toBeVisible();
  await tab(page, "Intake").click();
  // At half width the queue is a counter; the rows are there, hidden.
  await expect(page.getByRole("group", { name: "Intake counter" }).getByTestId("intake-position")).toHaveText(/\d/);
}

/** Chooses queued rows from `from` until one whose Stems can be split; its id, place and title. */
async function chooseSplittable(page: Page, from = 0): Promise<{ id: string; at: number; title: string }> {
  for (let at = from; at < from + 30; at += 1) {
    const row = options(page).nth(at);
    const title = await row.locator("span").first().innerText();
    await row.click();
    await expect(loadedTitle(page)).toHaveText(title);
    await expect(stems(page)).not.toHaveAttribute("data-state", /reading|empty/);
    if ((await stems(page).getAttribute("data-state")) === "none") {
      return { id: (await row.getAttribute("data-track"))!, at, title };
    }
  }
  throw new Error("no queued track with audio to split");
}

async function choose(page: Page, at: number): Promise<void> {
  const row = options(page).nth(at);
  const title = await row.locator("span").first().innerText();
  await row.click();
  await expect(loadedTitle(page)).toHaveText(title);
}

test("Split starts a separation shown as a Run with progress in the status line, and then the track has Stems", async ({ page }) => {
  await openIntake(page);
  const track = await chooseSplittable(page);
  await expect(stemsState(page)).toHaveText("No Stems yet");
  await expect(splitButton(page)).toHaveText("Split");

  await splitButton(page).click();

  const bar = stems(page).getByRole("progressbar", { name: "Splitting Stems" });
  await expect(bar).toBeVisible();
  await expect(stemsState(page)).toHaveText(/^Splitting: (loading model|separating|writing) \d+%$/);
  await expect(statusLine(page)).toContainText(`Separate track ${track.id} "${track.title}" into Stems`);
  await expect(statusLine(page).getByTestId("status-stems-running")).toContainText(`Stems · ${track.title} ·`);
  await expect.poll(async () => Number(await bar.getAttribute("aria-valuenow"))).toBeGreaterThan(20);

  await expect(stems(page)).toHaveAttribute("data-state", "current", { timeout: 10_000 });
  await expect(stemsState(page)).toHaveText(/^Stems ready · made \S.*\d/);
  await expect(splitButton(page)).toHaveText("Split again…");
  await expect(statusLine(page).getByTestId("status-stems")).toHaveCount(0);
  await expect(statusLine(page)).not.toContainText("into Stems");
  expect(await runs(page, track.id)).toBe(1);
});

test("a split blocks nothing: Mark Clean and next moves on, and the split carries on across views and tracks", async ({ page }) => {
  await openIntake(page);
  await call(page, "__holdStems");
  const track = await chooseSplittable(page);
  await splitButton(page).click();
  await expect(stems(page)).toHaveAttribute("data-state", "running");

  // Mark Clean and next does not wait for it.
  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
  await expect(loadedTitle(page)).not.toHaveText(track.title);
  await expect(queue(page).locator(`[data-track="${track.id}"]`)).toHaveCount(0);
  // Another track's control is its own; the status line still shows the split.
  await expect(stems(page)).not.toHaveAttribute("data-state", "running");
  await expect(statusLine(page).getByTestId("status-stems-running")).toContainText(track.title);

  // Across a view switch and back.
  await tab(page, "Library").click();
  await expect(statusLine(page).getByTestId("status-stems-running")).toContainText(track.title);
  await tab(page, "Intake").click();
  await expect(statusLine(page).getByTestId("status-stems-running")).toContainText(track.title);

  await call(page, "__releaseStems");
  await expect(statusLine(page).getByTestId("status-stems")).toHaveCount(0, { timeout: 10_000 });
  expect(await runs(page, track.id)).toBe(1);
});

test("a track being split offers no second split, by click, key or palette", async ({ page }) => {
  await openIntake(page);
  await call(page, "__holdStems");
  const track = await chooseSplittable(page);
  await splitButton(page).click();
  await expect(stems(page)).toHaveAttribute("data-state", "running");
  await expect(splitButton(page)).toHaveCount(0);

  // Away and back: still the same split, still no second.
  await choose(page, track.at + 1);
  await choose(page, track.at);
  await expect(stems(page)).toHaveAttribute("data-state", "running");
  await expect(splitButton(page)).toHaveCount(0);
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await page.keyboard.press("Control+Shift+P");
  await palette.getByRole("combobox", { name: "Search actions" }).fill("Split Stems");
  await expect(palette.getByRole("option", { name: /^Split Stems/ })).toHaveAttribute("aria-disabled", "true");
  await page.keyboard.press("Escape");

  await call(page, "__releaseStems");
  await expect(stems(page)).toHaveAttribute("data-state", "current", { timeout: 10_000 });
  expect(await runs(page, track.id)).toBe(1);
});

test("Split is a keyboard control that owns Space and Enter: the deck does not play", async ({ page }) => {
  await openIntake(page);
  await call(page, "__holdStems");
  await chooseSplittable(page);
  await splitButton(page).focus();
  await page.keyboard.press("Space");
  await expect(stems(page)).toHaveAttribute("data-state", "running");
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();
  await call(page, "__releaseStems");
});

test("a burst of progress is drawn a few times, not once per event", async ({ page }) => {
  await openIntake(page);
  await call(page, "__holdStems");
  await chooseSplittable(page);
  await splitButton(page).click();
  const bar = stems(page).getByRole("progressbar", { name: "Splitting Stems" });
  await expect(bar).toBeVisible();
  // Wait until the fake has finished its own reports and is held.
  await expect.poll(async () => Number(await bar.getAttribute("aria-valuenow")), { timeout: 10_000 }).toBe(90);
  await page.evaluate(() => {
    const bar = document.querySelector('[aria-label="Splitting Stems"]')!;
    const changes = { count: 0 };
    (window as unknown as { __barChanges: typeof changes }).__barChanges = changes;
    new MutationObserver((list) => { changes.count += list.length; }).observe(bar, { attributes: true, attributeFilter: ["aria-valuenow"] });
  });
  await call(page, "__stemsBurst", 100);
  await page.waitForTimeout(300);
  const changes = await page.evaluate(() => (window as unknown as { __barChanges: { count: number } }).__barChanges.count);
  expect(changes).toBeLessThanOrEqual(5);
  await expect(stemsState(page)).toHaveText("Splitting: separating 79%");
  await call(page, "__releaseStems");
});

test("current Stems are split again only after the DJ confirms", async ({ page }) => {
  await openIntake(page);
  const track = await chooseSplittable(page);
  await call(page, "__setStems", track.id, "current");
  await expect(stems(page)).toHaveAttribute("data-state", "current");
  await expect(splitButton(page)).toHaveText("Split again…");

  await page.evaluate(() => { (window as unknown as { __confirmAnswers: boolean[] }).__confirmAnswers = [false]; });
  await splitButton(page).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __confirmed?: string[] }).__confirmed?.length ?? 0)).toBe(1);
  await expect(stems(page)).toHaveAttribute("data-state", "current");
  expect(await runs(page, track.id)).toBe(0);

  await page.evaluate(() => { (window as unknown as { __confirmAnswers: boolean[] }).__confirmAnswers = [true]; });
  await splitButton(page).click();
  await expect(stems(page)).toHaveAttribute("data-state", "running");
  await expect(stems(page)).toHaveAttribute("data-state", "current", { timeout: 10_000 });
  expect(await runs(page, track.id)).toBe(1);
});

test("stale Stems are labelled and split without asking", async ({ page }) => {
  await openIntake(page);
  const track = await chooseSplittable(page);
  await call(page, "__setStems", track.id, "stale");
  await expect(stems(page)).toHaveAttribute("data-state", "stale");
  await expect(stemsState(page)).toHaveText(/^Stale: the audio changed since these Stems were made \([^)]*\d[^)]*\)$/);
  await expect(splitButton(page)).toHaveText("Split");
  await splitButton(page).click();
  await expect(stems(page)).toHaveAttribute("data-state", "current", { timeout: 10_000 });
  expect(await page.evaluate(() => (window as unknown as { __confirmed?: string[] }).__confirmed ?? [])).toEqual([]);
});

test("a failed split is reported with its reason on the control and in the status line, and can be retried", async ({ page }) => {
  await openIntake(page);
  await call(page, "__failStems", "the engine ran out of memory");
  const track = await chooseSplittable(page);
  await splitButton(page).click();

  await expect(stems(page)).toHaveAttribute("data-state", "failed", { timeout: 10_000 });
  await expect(stemsState(page)).toHaveText("Split failed: the engine ran out of memory");
  const failed = statusLine(page).getByTestId("status-stems-failed");
  await expect(failed).toContainText(`Stems failed for “${track.title}”: the engine ran out of memory`);
  // Still reported after moving to another track.
  await choose(page, track.at + 1);
  await expect(failed).toBeVisible();
  await choose(page, track.at);

  await call(page, "__failStems", null);
  await splitButton(page).click();
  await expect(stems(page)).toHaveAttribute("data-state", "current", { timeout: 10_000 });
  await expect(failed).toHaveCount(0);
  expect(await runs(page, track.id)).toBe(2);
});

test("a second track waits its turn behind the one being split", async ({ page }) => {
  await openIntake(page);
  await call(page, "__holdStems");
  const first = await chooseSplittable(page);
  await splitButton(page).click();
  const second = await chooseSplittable(page, first.at + 1);
  await splitButton(page).click();
  await expect(stems(page)).toHaveAttribute("data-state", "queued");
  await expect(stemsState(page)).toHaveText(`Queued behind “${first.title}”`);
  await expect(statusLine(page)).toContainText("+1 queued");
  expect(await runs(page, second.id)).toBe(0);

  await call(page, "__releaseStems");
  await expect(stems(page)).toHaveAttribute("data-state", "running", { timeout: 10_000 });
  await expect(stems(page)).toHaveAttribute("data-state", "current", { timeout: 10_000 });
  expect(await runs(page, first.id)).toBe(1);
  expect(await runs(page, second.id)).toBe(1);
});

test("a Stems read that fails is shown with Retry, not as an old state", async ({ page }) => {
  await openIntake(page);
  const track = await chooseSplittable(page);
  await call(page, "__setStems", track.id, "current");
  await expect(stems(page)).toHaveAttribute("data-state", "current");
  await call(page, "__failStemsReads", true);
  await call(page, "__setStems", track.id, "current");
  await expect(stems(page)).toHaveAttribute("data-state", "unread");
  await expect(stemsState(page)).toHaveText("Could not read Stems: the Sidecar could not be read");
  await expect(stemsState(page)).not.toContainText("ready");
  await call(page, "__failStemsReads", false);
  await stems(page).getByRole("button", { name: "Retry" }).click();
  await expect(stems(page)).toHaveAttribute("data-state", "current");
});

for (const width of [1280, 900, 640, 436]) {
  test(`at ${width}px the Stems control and the status line stay within the window`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await openIntake(page);
    if (width <= 800) {
      // The queue is a counter here: the loaded track comes from it.
      await page.getByRole("group", { name: "Intake counter" }).getByRole("button", { name: "Next track" }).click();
      await expect(stems(page)).toBeHidden();
      // Behind the palette instead.
      await call(page, "__holdStems");
      const palette = page.getByRole("dialog", { name: "Command palette" });
      let started = false;
      for (let tries = 0; tries < 10 && !started; tries += 1) {
        await page.keyboard.press("Control+Shift+P");
        await palette.getByRole("combobox", { name: "Search actions" }).fill("Split Stems");
        const entry = palette.getByRole("option", { name: /^Split Stems/ });
        if ((await entry.getAttribute("aria-disabled")) !== "true") {
          await entry.click();
          started = true;
        } else {
          await page.keyboard.press("Escape");
          await page.getByRole("group", { name: "Intake counter" }).getByRole("button", { name: "Next track" }).click();
        }
      }
      expect(started).toBe(true);
    } else {
      await call(page, "__holdStems");
      await chooseSplittable(page);
      await splitButton(page).click();
      await expect(stems(page)).toHaveAttribute("data-state", "running");
      const box = (await stems(page).boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(width + 0.5);
    }
    const running = statusLine(page).getByTestId("status-stems-running");
    await expect(running).toBeVisible();
    const line = (await statusLine(page).boundingBox())!;
    expect(line.x + line.width).toBeLessThanOrEqual(width + 0.5);
    const item = (await running.boundingBox())!;
    expect(item.x + item.width).toBeLessThanOrEqual(width + 0.5);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
    await call(page, "__releaseStems");
  });
}

test("under light themes the Stems control and the status line read in every state", async ({ page }) => {
  await openIntake(page);
  await call(page, "__holdStems");
  const running = await chooseSplittable(page);
  await splitButton(page).click();
  await expect(stems(page)).toHaveAttribute("data-state", "running");
  const stale = await chooseSplittable(page, running.at + 1);
  await call(page, "__setStems", stale.id, "stale");
  await expect(stems(page)).toHaveAttribute("data-state", "stale");
  for (const roles of [LATTE, WHITE]) {
    await announce(page, { roles, font: null });
    await expect(page.locator("html")).toHaveAttribute("data-theme-mode", "light");
    for (const region of [stems(page), statusLine(page)]) {
      const { seen } = await audit(region);
      expect(seen.length).toBeGreaterThan(1);
      expect([...new Set(shortfalls(seen))], roles.working).toEqual([]);
    }
  }
  // A failure, too.
  await call(page, "__failStems", "the engine ran out of memory");
  await call(page, "__releaseStems");
  await choose(page, running.at);
  await expect(stems(page)).toHaveAttribute("data-state", "failed", { timeout: 10_000 });
  for (const roles of [LATTE, WHITE]) {
    await announce(page, { roles, font: null });
    for (const region of [stems(page), statusLine(page)]) {
      expect([...new Set(shortfalls((await audit(region)).seen))], roles.working).toEqual([]);
    }
  }
});
