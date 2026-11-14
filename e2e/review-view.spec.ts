/**
 * The Review view (wbx-74): the Facts waiting for the DJ, grouped by Run,
 * accepted, rejected and reopened through the service's own commands
 * (wbx-27), with nothing added beyond selection and the threshold.
 *
 * Mock backend: `src/ipc/mock-review.ts` (three Runs; Run 41 "tagging
 * pass" still working; Run 38 held back below its confidence; Run 40 Prep
 * colours). Its hooks add rows live, change a row, refuse the next accept,
 * and list every decision asked, with the token it carried.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { pauseButton, player, playButton, seedShell } from "./compactPlayer";

interface Call { command: string; facts: number[]; token?: string; rejection?: number | null; source: { kind: string } }

const tabs = (page: Page) => page.getByRole("tablist", { name: "Views" });
const tab = (page: Page, name: string) => tabs(page).getByRole("tab", { name });
const tabCount = (page: Page) => page.getByTestId("shell-tab-count-review");
const view = (page: Page) => page.getByTestId("review-view");
const run = (page: Page, id: number) => view(page).locator(`[role="group"][data-run="${id}"]`);
const rows = (scope: Locator) => scope.locator('[role="row"][data-fact]');
const row = (page: Page, fact: number) => view(page).locator(`[role="row"][data-fact="${fact}"]`);
const hook = <T,>(page: Page, name: string, ...args: unknown[]) =>
  page.evaluate(([n, a]) => (window as unknown as Record<string, (...x: unknown[]) => T>)[n]!(...(a)), [name, args] as const);
const calls = (page: Page) => hook<Call[]>(page, "__reviewCalls");
const factsOf = async (scope: Locator) => (await rows(scope).evaluateAll((els) => els.map((e) => Number(e.getAttribute("data-fact")))));

async function openReview(page: Page, query = ""): Promise<void> {
  await seedShell(page);
  await page.goto(`/${query}`);
  await expect(page.getByTestId("shell")).toBeVisible();
  await tab(page, "Review").click();
  await expect(tab(page, "Review")).toHaveAttribute("aria-selected", "true");
  await expect(view(page)).toBeVisible();
}

test("waiting Facts are listed by Run with track, field, now, proposed and confidence; the tab shows how many", async ({ page }) => {
  await openReview(page);
  await expect(rows(view(page))).toHaveCount(14);
  await expect(tabCount(page)).toHaveText("14");
  // Grouped by Run, lowest first.
  const groups = await view(page).locator('[role="group"][data-run]').evaluateAll((els) => els.map((e) => e.getAttribute("aria-label")));
  expect(groups).toEqual(["Run 38 · Beatport lookup", "Run 40 · Prep colour from analysis", "Run 41 · tagging pass"]);
  // A row: track, field, now, proposed, confidence.
  const cells = await rows(run(page, 41)).first().getByRole("cell").allInnerTexts();
  expect(cells[2]).toBe("Energy");
  expect(cells[4]).toBe("1");
  expect(cells[5]).toContain("0.91");
  expect(cells[1]!.length).toBeGreaterThan(0);
  // A Prep colour proposal reads as the DJ's label.
  await expect(rows(run(page, 40)).first().getByRole("cell").nth(4)).toHaveText("Clean");
  // The working Run says so.
  await expect(run(page, 41).getByTestId("review-run-working")).toBeVisible();
  // Reading wrote nothing and decided nothing.
  expect(await calls(page)).toEqual([]);
});

test("low-confidence rows stand out by more than colour, and held-back Facts show why", async ({ page }) => {
  await openReview(page);
  const low = view(page).locator('[role="row"][data-low]');
  await expect(low).toHaveCount(3);
  for (const r of await low.all()) {
    await expect(r).toContainText("low");
    const sure = Number((await r.getByRole("cell").nth(5).innerText()).match(/\d\.\d\d/)![0]);
    expect(sure).toBeLessThan(0.6);
  }
  // The high-confidence row is drawn differently: its edge is not the low one's.
  const edge = (r: Locator) => r.evaluate((el) => getComputedStyle(el).borderLeftColor);
  expect(await edge(low.first())).not.toBe(await edge(rows(run(page, 41)).first()));
  // Held back below the Run's confidence: the reason on the row and the Run.
  await expect(rows(run(page, 38)).first()).toContainText("held: below 0.70");
  await expect(run(page, 38)).toContainText("3 held: below 0.70");
});

test("accepting one row writes its value as the DJ's, with the token of the listing shown, and the row leaves", async ({ page }) => {
  await openReview(page, "?writable");
  const first = rows(run(page, 41)).first();
  const fact = Number(await first.getAttribute("data-fact"));
  const track = await first.getAttribute("data-track");
  const next = rows(run(page, 41)).nth(1);
  const nextBox = await next.boundingBox();

  await first.getByRole("button", { name: /^Accept / }).click();

  await expect(view(page).getByTestId("review-outcome")).toHaveAttribute("data-outcome", "done");
  await expect(view(page).getByTestId("review-outcome")).toContainText("Accepted 1 Fact");
  const asked = await calls(page);
  expect(asked).toEqual([{ command: "review.accept", facts: [fact], token: "mock.1", source: { kind: "dj" } }]);
  expect(await hook<number>(page, "__ratingOf", track)).toBe(1);
  await expect(tabCount(page)).toHaveText("13");
  // While the pointer is still on the list the decided row holds its place,
  // marked and without controls, so the next row does not slide under it.
  await expect(first).toHaveAttribute("data-ghost", "accepted");
  await expect(first.getByRole("button")).toHaveCount(0);
  expect(await next.boundingBox()).toEqual(nextBox);
  // Off the list (after the pointer leaves it).
  await page.mouse.move(2, 2);
  await expect(row(page, fact)).toHaveCount(0);
});

test("the bulk control says how many it would accept, and accepts exactly those", async ({ page }) => {
  await openReview(page, "?writable");
  const tagging = run(page, 41);
  const bulk = tagging.getByTestId("review-bulk-accept");
  await expect(bulk).toHaveText("Accept ≥ 0.80 · 5");
  await tagging.getByRole("spinbutton", { name: /Confidence for Run 41/ }).fill("0.7");
  await expect(bulk).toHaveText("Accept ≥ 0.70 · 6");
  const expected = await rows(tagging).evaluateAll((els) => els
    .filter((e) => Number((e.children[5] as HTMLElement).innerText.match(/\d\.\d\d/)![0]) >= 0.7)
    .map((e) => Number(e.getAttribute("data-fact"))));
  expect(expected).toHaveLength(6);

  await bulk.click();

  await expect(view(page).getByTestId("review-outcome")).toContainText("Accepted 6 Facts");
  const asked = await calls(page);
  expect(asked).toHaveLength(1);
  expect(asked[0]!.facts.sort()).toEqual(expected.sort());
  await page.mouse.move(2, 2);
  await expect(rows(run(page, 41))).toHaveCount(3);
});

test("a selection is accepted or rejected as one decision, and only what was chosen", async ({ page }) => {
  await openReview(page, "?writable");
  const [a, b] = [rows(run(page, 41)).nth(1), rows(run(page, 38)).nth(0)];
  const ids = [Number(await a.getAttribute("data-fact")), Number(await b.getAttribute("data-fact"))];
  await a.getByRole("checkbox").check();
  await b.getByRole("checkbox").check();
  await expect(view(page).getByTestId("review-chosen")).toContainText("2 Facts chosen");

  await view(page).getByRole("button", { name: "Reject chosen · 2" }).click();

  await expect(view(page).getByTestId("review-outcome")).toContainText("Rejected 2 Facts");
  expect((await calls(page)).map((c) => [c.command, [...c.facts].sort(), c.source.kind])).toEqual([["review.reject", [...ids].sort(), "dj"]]);
  await expect(tabCount(page)).toHaveText("12");
});

test("a mistaken bulk rejection is reopened from the view, as a whole or row by row", async ({ page }) => {
  await openReview(page);
  const tagging = run(page, 41);
  await tagging.getByTestId("review-bulk-reject").click();
  await expect(view(page).getByTestId("review-outcome")).toContainText("Rejected 5 Facts");
  await expect(tabCount(page)).toHaveText("9");

  await view(page).getByRole("button", { name: "Reopen these 5" }).click();

  await expect(tabCount(page)).toHaveText("14");
  const reopen = (await calls(page)).at(-1)!;
  expect(reopen.command).toBe("review.reopen");
  expect(reopen.source).toEqual({ kind: "dj" });
  expect(typeof reopen.rejection).toBe("number");

  // Row by row from the rejected list.
  const one = rows(run(page, 38)).first();
  const fact = Number(await one.getAttribute("data-fact"));
  await one.getByRole("button", { name: /^Reject / }).click();
  await expect(tabCount(page)).toHaveText("13");
  await view(page).getByTestId("review-rejected").locator("summary").click();
  const listed = view(page).getByTestId("review-rejected").locator(`li[data-fact="${fact}"]`);
  await listed.getByRole("button", { name: /^Reopen / }).click();
  await expect(tabCount(page)).toHaveText("14");
  await expect(listed).toHaveCount(0);
});

test("rows arriving live never move the row under the pointer nor change what the bulk control covers", async ({ page }) => {
  await openReview(page);
  const tagging = run(page, 41);
  const bulk = tagging.getByTestId("review-bulk-accept");
  await expect(bulk).toHaveText("Accept ≥ 0.80 · 5");
  const before = await factsOf(view(page));
  // The pointer rests on the bulk control.
  await bulk.hover();
  const box = await bulk.boundingBox();

  await hook(page, "__reviewArrive", 5);
  await expect(view(page).getByTestId("review-pending")).toContainText("5 new");

  // Nothing moved, nothing was added, the count is the same.
  expect(await factsOf(view(page))).toEqual(before);
  expect(await bulk.boundingBox()).toEqual(box);
  await expect(bulk).toHaveText("Accept ≥ 0.80 · 5");
  // The tab follows at once.
  await expect(tabCount(page)).toHaveText("19");

  // Used now, it sends exactly the 5 shown, with the shown listing's token;
  // the service refuses (the listing changed) and the view shows the new
  // rows, marked, for the DJ to decide again. Nothing is retried.
  const shown = await rows(tagging).evaluateAll((els) => els
    .filter((e) => Number((e.children[5] as HTMLElement).innerText.match(/\d\.\d\d/)![0]) >= 0.8)
    .map((e) => Number(e.getAttribute("data-fact"))));
  await bulk.click();
  await expect(view(page).getByTestId("review-outcome")).toHaveAttribute("data-code", "review_changed");
  const asked = await calls(page);
  expect(asked).toHaveLength(1);
  expect([...asked[0]!.facts].sort()).toEqual([...shown].sort());
  expect(asked[0]!.token).toBe("mock.1");
  await expect(view(page).getByTestId("review-changes")).toContainText("5 new");
  await expect(view(page).locator('[role="row"][data-marked="added"]')).toHaveCount(5);
  await expect(rows(view(page))).toHaveCount(19);
  // Deciding again uses the new listing.
  await page.waitForTimeout(300);
  expect(await calls(page)).toHaveLength(1);
});

test("rows arrive live while the pointer is away from the list", async ({ page }) => {
  await openReview(page);
  await page.mouse.move(2, 2);
  await hook(page, "__reviewArrive", 3);
  await expect(rows(run(page, 41))).toHaveCount(12);
  await expect(tabCount(page)).toHaveText("17");
  // A burst arrives as a few reads, not one per Fact.
  const readsBefore = (await hook<{ waiting: number }>(page, "__reviewReads")).waiting;
  await hook(page, "__reviewArrive", 1);
  await hook(page, "__reviewArrive", 1);
  await hook(page, "__reviewArrive", 1);
  await expect(tabCount(page)).toHaveText("20");
  const readsAfter = (await hook<{ waiting: number }>(page, "__reviewReads")).waiting;
  expect(readsAfter - readsBefore).toBeLessThanOrEqual(3);
  // The Run finishing is seen too.
  await hook(page, "__reviewFinishRun");
  await expect(run(page, 41).getByTestId("review-run-working")).toHaveCount(0);
});

test("a part-way accept is shown as what was applied and what was not, never as success", async ({ page }) => {
  await openReview(page, "?writable");
  const a = rows(run(page, 41)).nth(0);
  const b = rows(run(page, 41)).nth(3);
  const [fa, fb] = [Number(await a.getAttribute("data-fact")), Number(await b.getAttribute("data-fact"))];
  await a.getByRole("checkbox").check();
  await b.getByRole("checkbox").check();
  await hook(page, "__reviewNext", "review_partial");

  await view(page).getByRole("button", { name: "Accept chosen · 2" }).click();

  const outcome = view(page).getByTestId("review-outcome");
  await expect(outcome).toHaveAttribute("data-outcome", "partial");
  await expect(outcome).toHaveAttribute("role", "alert");
  await expect(outcome.getByTestId("review-applied")).toContainText(`Fact ${fa}`);
  await expect(outcome.getByTestId("review-not-applied")).toContainText(`Fact ${fb}`);
  await expect(outcome).not.toContainText("Accepted");
  await page.mouse.move(2, 2);
  await expect(row(page, fb)).toHaveCount(1);
  await expect(row(page, fa)).toHaveCount(0);
});

test("a refused accept (Library Protection) is said, and the Fact still waits", async ({ page }) => {
  await openReview(page);
  const first = rows(run(page, 41)).first();
  const fact = Number(await first.getAttribute("data-fact"));
  await hook(page, "__reviewNext", "library_protected");
  await first.getByRole("button", { name: /^Accept / }).click();
  await expect(view(page).getByTestId("review-outcome")).toHaveAttribute("data-code", "library_protected");
  await expect(view(page).getByTestId("review-outcome")).toContainText("Nothing was accepted");
  await expect(row(page, fact)).toHaveCount(1);
  await expect(tabCount(page)).toHaveText("14");
});

test("Listen loads the row's track into the deck and plays it, staying in the view", async ({ page }) => {
  await openReview(page);
  const first = rows(run(page, 41)).nth(3);
  const title = await first.getByRole("cell").nth(1).innerText();
  await first.getByRole("button", { name: /^Listen to / }).click();
  await expect(player(page).getByTestId("compact-player-title")).toHaveText(title);
  await expect(pauseButton(page)).toBeVisible();
  await expect(tab(page, "Review")).toHaveAttribute("aria-selected", "true");
  await expect(first).toHaveAttribute("data-loaded", "true");
});

test("Space and Enter press the view's controls and never reach the deck", async ({ page }) => {
  await openReview(page, "?writable");
  const first = rows(run(page, 41)).nth(3);
  await first.getByRole("button", { name: /^Listen to / }).click();
  await expect(pauseButton(page)).toBeVisible();
  // Keyboard: the second row's checkbox by Space, its Accept by Enter.
  const second = rows(run(page, 41)).nth(1);
  const fact = Number(await second.getAttribute("data-fact"));
  await second.getByRole("checkbox").focus();
  await page.keyboard.press(" ");
  await expect(second.getByRole("checkbox")).toBeChecked();
  await second.getByRole("button", { name: /^Accept / }).focus();
  await page.keyboard.press("Enter");
  await expect(view(page).getByTestId("review-outcome")).toContainText("Accepted 1 Fact");
  expect((await calls(page)).map((c) => c.facts)).toEqual([[fact]]);
  // The deck played on throughout.
  await expect(pauseButton(page)).toBeVisible();
  await expect(playButton(page)).toHaveCount(0);
});

test("with nothing waiting the view says so, and the tab shows none", async ({ page }) => {
  await openReview(page, "?review=empty");
  await expect(view(page).getByTestId("review-empty")).toContainText("Nothing is waiting for you");
  await expect(tabCount(page)).toHaveText("0");
});

test("a read that fails is shown with Retry, not hidden behind the last listing", async ({ page }) => {
  await openReview(page);
  await expect(rows(view(page))).toHaveCount(14);
  await hook(page, "__failReviewReads", true);
  await page.mouse.move(2, 2);
  await hook(page, "__reviewArrive", 1);
  await expect(view(page).getByTestId("review-read-failed")).toContainText("could not be read");
  await expect(view(page).getByTestId("review-read-failed")).toContainText("may be out of date");
  await expect(tabCount(page)).toHaveCount(0);
  await hook(page, "__failReviewReads", false);
  await view(page).getByTestId("review-read-failed").getByRole("button", { name: "Retry" }).click();
  await expect(view(page).getByTestId("review-read-failed")).toHaveCount(0);
  await expect(rows(view(page))).toHaveCount(15);
});

for (const width of [1280, 900, 640, 436]) {
  test(`at ${width}px the view stays within the window and its controls can be reached`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await openReview(page);
    await expect(rows(view(page))).toHaveCount(14);
    const box = await view(page).boundingBox();
    expect(box!.x + box!.width).toBeLessThanOrEqual(width + 0.5);
    const overflow = await view(page).evaluate((el) => {
      const list = el.querySelector('[data-testid="review-list"]') as HTMLElement;
      return { page: document.documentElement.scrollWidth - document.documentElement.clientWidth, list: list.scrollWidth - list.clientWidth };
    });
    expect(overflow.page).toBeLessThanOrEqual(0);
    expect(overflow.list).toBeLessThanOrEqual(0);
    // Every row's decision buttons are inside the view.
    for (const button of await rows(view(page)).getByRole("button", { name: /^Reject / }).all()) {
      const b = await button.boundingBox();
      expect(b!.x + b!.width, `${width}`).toBeLessThanOrEqual(box!.x + box!.width + 0.5);
    }
    const bulk = run(page, 41).getByTestId("review-bulk-accept");
    const bb = await bulk.boundingBox();
    expect(bb!.x + bb!.width).toBeLessThanOrEqual(box!.x + box!.width + 0.5);
  });
}
