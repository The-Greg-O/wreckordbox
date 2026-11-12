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
  await expect(page.getByTestId("intake-panel-title")).toHaveText(title);

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
  await options(page).nth(2).click();
  await expect(detail(page)).toBeVisible();
  await expect(options(page).nth(2)).toBeVisible();
  const box = await detail(page).boundingBox();
  expect(box!.width).toBeLessThanOrEqual(640);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(640);
});

