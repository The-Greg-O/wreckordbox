/**
 * Intake at half width (wbx-69, docs/design/screens/intake "Sizes"): tiled
 * beside a terminal the DJ can still finish a track. At 800px and below the
 * queue becomes a counter ("2 of 23", what the track needs, previous and
 * next), the large player stays, the tag panel goes from three columns to
 * two with every tag reachable, and the grid tools move behind the command
 * palette. Nothing overlaps or is cut off.
 *
 * Mock backend (`src/ipc/mock-intake.ts`, `mock-tag-panel.ts`,
 * `mock-cue-pads.ts`).
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { player, seedShell } from "./compactPlayer";

type Hooks = Record<string, (...args: unknown[]) => unknown>;
type Grid = { beatMs: number; bpmX100: number; locked: boolean } | null;

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const queue = (page: Page) => page.getByRole("listbox", { name: "Intake queue" });
const options = (page: Page) => queue(page).getByRole("option");
const counter = (page: Page) => page.getByRole("group", { name: "Intake counter" });
const nextTrack = (page: Page) => counter(page).getByRole("button", { name: "Next track", exact: true });
const previousTrack = (page: Page) => counter(page).getByRole("button", { name: "Previous track", exact: true });
const position = (page: Page) => page.getByTestId("intake-position");
const counterNeeds = (page: Page) => counter(page).getByTestId("intake-counter-needs");
const loadedTitle = (page: Page) => player(page).getByTestId("compact-player-title");
const detail = (page: Page) => player(page).getByTestId("large-player-detail");
const panel = (page: Page) => page.getByRole("region", { name: "Track" });
const tags = (page: Page) => page.getByTestId("intake-tags");
const titleField = (page: Page) => page.getByRole("textbox", { name: "Title" });
const playButton = (page: Page) => player(page).getByRole("button", { name: "Play", exact: true });
const grid = (page: Page) => player(page).getByRole("group", { name: "Beat grid" });
const pads = (page: Page) => player(page).getByRole("group", { name: "Hot cue pads" });

const call = (page: Page, name: string, ...args: unknown[]) =>
  page.evaluate(([hook, rest]) => (window as unknown as Hooks)[hook]!(...(rest)), [name, args] as const);
const gridOf = async (page: Page, id: string) => (await call(page, "__gridOf", id)) as Grid;

async function openIntake(page: Page, width: number, query = "?tagSchema=five&writable=1", view: Record<string, unknown> = {}): Promise<void> {
  await page.setViewportSize({ width, height: 800 });
  await seedShell(page, view);
  await page.goto(`/${query}`);
  await expect(page.getByTestId("shell")).toBeVisible();
  await tab(page, "Intake").click();
  await expect(page.getByTestId("intake-view")).toBeVisible();
}

/** The queued track the view shows as chosen, by its title in the large player. */
async function nextLoads(page: Page): Promise<string> {
  const before = await loadedTitle(page).innerText().catch(() => "");
  await nextTrack(page).click();
  await expect(loadedTitle(page)).not.toHaveText(before);
  await expect(titleField(page)).toHaveValue(await loadedTitle(page).innerText());
  return loadedTitle(page).innerText();
}

type Box = { x: number; y: number; width: number; height: number };
const overlaps = (a: Box, b: Box) =>
  a.x < b.x + b.width - 0.5 && b.x < a.x + a.width - 0.5 && a.y < b.y + b.height - 0.5 && b.y < a.y + a.height - 0.5;

/**
 * Every visible element of `selector` inside `scope` lies within [0, width],
 * and none covers another (an element and what it contains aside).
 */
async function expectLaidOut(scope: Locator, selector: string, width: number): Promise<void> {
  const { boxes, nested } = await scope.first().evaluate((root, sel) => {
    const shown = [...root.querySelectorAll(sel)].filter((element) => element.getClientRects().length > 0
      && getComputedStyle(element).visibility !== "hidden" && Number(getComputedStyle(element).opacity) > 0);
    return {
      boxes: shown.map((element) => { const r = element.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height, name: element.textContent?.trim() ?? "" }; }),
      nested: shown.flatMap((a, i) => shown.flatMap((b, j) => (i !== j && a.contains(b) ? [`${i}:${j}`, `${j}:${i}`] : []))),
    };
  }, selector);
  expect(boxes.length).toBeGreaterThan(0);
  const inside = new Set(nested);
  for (const box of boxes) {
    expect(box.x, box.name).toBeGreaterThanOrEqual(-0.5);
    expect(box.x + box.width, box.name).toBeLessThanOrEqual(width + 0.5);
  }
  for (let i = 0; i < boxes.length; i++) {
    for (let j = i + 1; j < boxes.length; j++) {
      if (inside.has(`${i}:${j}`)) continue;
      expect(overlaps(boxes[i]!, boxes[j]!), `${boxes[i]!.name} over ${boxes[j]!.name}`).toBe(false);
    }
  }
}

/** Distinct left edges of the tag panel's dimensions: its columns. */
async function tagColumns(page: Page): Promise<number> {
  const lefts = await tags(page).getByRole("group").evaluateAll((groups) => groups.map((g) => Math.round(g.getBoundingClientRect().x)));
  return new Set(lefts).size;
}

test("at 640px the queue is a counter with previous and next, and the large player stays", async ({ page }) => {
  await openIntake(page, 640);
  await expect(queue(page)).toBeHidden();
  await expect(page.getByRole("navigation", { name: "Queue pages" })).toBeHidden();
  await expect(counter(page)).toBeVisible();
  await expect(previousTrack(page)).toBeDisabled();
  const total = (await position(page).innerText()).trim();

  const first = await nextLoads(page);
  await expect(position(page)).toHaveText(`1 of ${total}`);
  await expect(detail(page)).toBeVisible();
  await expect(previousTrack(page)).toBeDisabled();
  await expect(counterNeeds(page)).toHaveText(/^(needs .+|nothing missing)$/);

  const second = await nextLoads(page);
  expect(second).not.toBe(first);
  await expect(position(page)).toHaveText(`2 of ${total}`);
  await expect(previousTrack(page)).toBeEnabled();

  await previousTrack(page).click();
  await expect(loadedTitle(page)).toHaveText(first);
  await expect(position(page)).toHaveText(`1 of ${total}`);
});

test("the counter's needs are the loaded track's, as the queue row says them", async ({ page }) => {
  await openIntake(page, 1280);
  const row = options(page).nth(1);
  const needs = await row.getByTestId("intake-needs").innerText();
  await row.click();
  const title = await loadedTitle(page).innerText();
  await page.setViewportSize({ width: 640, height: 800 });
  // The choice survives the window narrowing: same track, its place and needs.
  await expect(loadedTitle(page)).toHaveText(title);
  await expect(position(page)).toHaveText(/^2 of \d+$/);
  await expect(counterNeeds(page)).toHaveText(needs);
});

test("next from a page's last track goes on to the next page, and previous comes back", async ({ page }) => {
  await openIntake(page, 1280);
  await options(page).first().click();
  await queue(page).press("End");
  await expect(position(page)).toHaveText(/^200 of \d+$/);
  const last = await loadedTitle(page).innerText();
  await page.setViewportSize({ width: 640, height: 800 });
  await nextLoads(page);
  await expect(position(page)).toHaveText(/^201 of \d+$/);
  await previousTrack(page).click();
  await expect(loadedTitle(page)).toHaveText(last);
  await expect(position(page)).toHaveText(/^200 of \d+$/);
});

test("the counter's buttons own Space and Enter: the deck never plays", async ({ page }) => {
  await openIntake(page, 640);
  await nextLoads(page);
  const title = await loadedTitle(page).innerText();
  await nextTrack(page).focus();
  await page.keyboard.press("Space");
  await expect(loadedTitle(page)).not.toHaveText(title);
  const second = await loadedTitle(page).innerText();
  await page.keyboard.press("Enter");
  await expect(loadedTitle(page)).not.toHaveText(second);
  await page.waitForTimeout(300);
  await expect(playButton(page)).toBeVisible();
});

test("at full width the queue is the list, and the counter's buttons are not drawn", async ({ page }) => {
  await openIntake(page, 1280);
  await expect(options(page).first()).toBeVisible();
  await expect(nextTrack(page)).toBeHidden();
  await options(page).nth(1).click();
  await expect(tags(page).getByRole("group").first()).toBeVisible();
  expect(await tagColumns(page)).toBe(3);
});

for (const width of [640, 436]) {
  test(`at ${width}px the tags go to two columns and every tag is reachable and can be set`, async ({ page }) => {
    await openIntake(page, width);
    await nextLoads(page);
    await expect(tags(page).getByRole("group").first()).toBeVisible();
    expect(await tagColumns(page)).toBe(2);
    await expectLaidOut(tags(page), "button[data-tag]", width);
    // The last tag of the last dimension: scrolled to and set by the pointer.
    const last = tags(page).getByRole("group").last().locator("button[data-tag]").last();
    await last.scrollIntoViewIfNeeded();
    const box = (await last.boundingBox())!;
    const scroller = (await panel(page).boundingBox())!;
    expect(box.y).toBeGreaterThanOrEqual(scroller.y);
    expect(box.y + box.height).toBeLessThanOrEqual(scroller.y + scroller.height + 0.5);
    const was = await last.getAttribute("aria-pressed");
    await last.click();
    await expect(last).not.toHaveAttribute("aria-pressed", was ?? "false");
  });

  test(`at ${width}px nothing overlaps or is cut off`, async ({ page }) => {
    await openIntake(page, width);
    await nextLoads(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
    // The shell's areas, then the view's parts, side by side or stacked, never on each other.
    for (const id of ["shell-top-bar", "shell-player", "shell-working", "shell-status"]) {
      const b = (await page.getByTestId(id).boundingBox())!;
      expect(b.x + b.width, id).toBeLessThanOrEqual(width + 0.5);
    }
    await expectLaidOut(page.getByTestId("shell-top-bar"), '[role="tab"], button', width);
    await expectLaidOut(page.getByTestId("shell-status"), "button, span", width);
    await expectLaidOut(counter(page), "button, h2, [data-testid]", width);
    await expectLaidOut(pads(page), "button", width);
    await expectLaidOut(panel(page), 'input, [role="radio"]', width);
    await expectLaidOut(page.getByTestId("intake-actions"), "button, kbd, [data-testid]", width);
    await expectLaidOut(page.getByTestId("shell-working"), '[aria-label="Intake counter"], [data-testid="intake-panel"]', width);
    await expectLaidOut(player(page), '[data-testid="compact-player"], [data-testid="large-player-detail"], [data-testid="intake-cue-row"]', width);
    // The readouts are whole, not clipped by their boxes.
    for (const id of ["intake-position", "intake-counter-needs"]) {
      const clipped = await page.getByTestId(id).evaluate((el) => el.scrollWidth > el.clientWidth + 1 && getComputedStyle(el).textOverflow !== "ellipsis");
      expect(clipped, id).toBe(false);
    }
    await expect(page.getByRole("button", { name: /Mark Clean and next/ })).toBeInViewport();
  });
}

test("at half width the grid tools are behind the palette, and its Shift Beatgrid still moves the grid", async ({ page }) => {
  await openIntake(page, 1280);
  // A queued track whose grid can be edited, chosen at full width where the
  // grid tools say so; then the window narrows.
  let id: string | null = null;
  for (let at = 0; at < 30 && id === null; at += 1) {
    const candidate = (await options(page).nth(at).getAttribute("data-track"))!;
    const g = await gridOf(page, candidate);
    if (g === null || g.locked) continue;
    await options(page).nth(at).click();
    await expect(grid(page)).toHaveAttribute("data-ready");
    if (await grid(page).getByRole("button", { name: "Shift the grid later", exact: true }).isEnabled()) id = candidate;
  }
  expect(id).not.toBeNull();
  await page.setViewportSize({ width: 640, height: 800 });
  await expect(pads(page)).toBeVisible();
  await expect(grid(page)).toBeHidden();
  const before = (await gridOf(page, id!))!.beatMs;
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await page.keyboard.press("Control+Shift+P");
  await palette.getByRole("combobox", { name: "Search actions" }).fill("Shift Beatgrid right");
  const entry = palette.getByRole("listbox", { name: "Actions" }).getByRole("option", { name: /^Shift Beatgrid right\s*Player A/ });
  await expect(entry).not.toHaveAttribute("aria-disabled", "true");
  await entry.click();
  await expect(palette).toHaveCount(0);
  await expect.poll(async () => (await gridOf(page, id!))!.beatMs).toBe(before + 1);
});

test("at 125% interface scale a 1000px window is half width: the counter shows", async ({ page }) => {
  await openIntake(page, 1000, "?tagSchema=five&writable=1", { uiScale: 6 });
  await expect(counter(page)).toBeVisible();
  await expect(queue(page)).toBeHidden();
  await nextLoads(page);
  expect(await tagColumns(page)).toBe(2);
});
