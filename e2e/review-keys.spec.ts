/**
 * Y and N in the Review view (wbx-mvr; decisions.md 2026-11-14 item 5):
 * Y accepts and N rejects the ONE row the keyboard focus is in, bound to
 * that row's token exactly as its ✓ and ✕ are; never a bulk selection,
 * never a key's repeat or a key held from before the row had focus, never
 * a row that arrived, moved or changed since it took focus (the view says
 * why it did nothing), never from a text field, the palette or a dialog,
 * and never reaching the deck.
 *
 * Mock backend: `src/ipc/mock-review.ts` (see `review-view.spec.ts`).
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { elapsedSeconds, pauseButton, seedShell } from "./compactPlayer";

interface Call { command: string; facts: number[]; rows?: { fact: number; token: string }[]; source: { kind: string } }

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const view = (page: Page) => page.getByTestId("review-view");
const run = (page: Page, id: number) => view(page).locator(`[role="group"][data-run="${id}"]`);
const rows = (scope: Locator) => scope.locator('[role="row"][data-fact]');
const hook = <T,>(page: Page, name: string, ...args: unknown[]) =>
  page.evaluate(([n, a]) => (window as unknown as Record<string, (...x: unknown[]) => T>)[n]!(...(a)), [name, args] as const);
const calls = (page: Page) => hook<Call[]>(page, "__reviewCalls");
const refusal = (page: Page) => view(page).getByTestId("review-key-refused");
const factOf = async (r: Locator) => Number(await r.getAttribute("data-fact"));

async function openReview(page: Page, query = "?writable"): Promise<void> {
  await seedShell(page);
  await page.goto(`/${query}`);
  // The first page of a run waits on the dev server compiling the app.
  await expect(page.getByTestId("shell")).toBeVisible({ timeout: 20_000 });
  await tab(page, "Review").click();
  await expect(view(page)).toBeVisible();
  await expect(rows(view(page))).toHaveCount(14);
}

/** Keyboard focus into a row: Tab from the control before it, as the DJ would. */
async function tabInto(page: Page, r: Locator): Promise<void> {
  await r.getByRole("checkbox").focus();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  await expect(r.getByRole("checkbox")).toBeFocused();
}

/** Nothing more is asked of the service for a moment. */
async function nothingAsked(page: Page, before = 0): Promise<void> {
  await page.waitForTimeout(300);
  expect(await calls(page)).toHaveLength(before);
}

test("Y accepts the focused row and N rejects the focused row, each with that row's own token, as its buttons do", async ({ page }) => {
  await openReview(page);
  const first = rows(run(page, 41)).first();
  const fact = await factOf(first);
  await tabInto(page, first);
  await page.keyboard.press("y");
  await expect(view(page).getByTestId("review-outcome")).toContainText("Accepted 1 Fact");
  expect(await calls(page)).toEqual([{ command: "review.accept", facts: [fact], rows: [{ fact, token: `r${fact}.0` }], source: { kind: "dj" } }]);

  const other = rows(run(page, 38)).first();
  const rejected = await factOf(other);
  await tabInto(page, other);
  await page.keyboard.press("n");
  await expect(view(page).getByTestId("review-outcome")).toContainText("Rejected 1 Fact");
  const asked = await calls(page);
  expect(asked).toHaveLength(2);
  expect(asked[1]).toMatchObject({ command: "review.reject", facts: [rejected], source: { kind: "dj" } });
});

test("Y on a row that is part of a selection decides that row only, never the selection", async ({ page }) => {
  await openReview(page);
  const tagging = rows(run(page, 41));
  await tagging.nth(1).getByRole("checkbox").check();
  await tagging.nth(2).getByRole("checkbox").check();
  await expect(view(page).getByTestId("review-chosen")).toContainText("2 Facts chosen");
  const focused = tagging.nth(1);
  const fact = await factOf(focused);
  await tabInto(page, focused);
  await page.keyboard.press("y");
  await expect(view(page).getByTestId("review-outcome")).toContainText("Accepted 1 Fact");
  expect((await calls(page)).map((c) => c.facts)).toEqual([[fact]]);
});

test("a held Y decides once; its repeats do nothing", async ({ page }) => {
  await openReview(page);
  const first = rows(run(page, 41)).first();
  await tabInto(page, first);
  await page.keyboard.down("y");
  for (let i = 0; i < 5; i += 1) await page.keyboard.down("y");
  await page.keyboard.up("y");
  await expect(view(page).getByTestId("review-outcome")).toContainText("Accepted 1 Fact");
  await nothingAsked(page, 1);
});

test("a Y held from before the row had focus does nothing, says why, and a fresh press then acts", async ({ page }) => {
  await openReview(page);
  const first = rows(run(page, 41)).first();
  const fact = await factOf(first);
  // Down on the row before, then the focus moves on with the key still held.
  const before = rows(run(page, 38)).first();
  await tabInto(page, before);
  await page.keyboard.down("y");
  await expect.poll(async () => (await calls(page)).length).toBe(1);
  await first.getByRole("checkbox").focus();
  for (let i = 0; i < 3; i += 1) await page.keyboard.down("y");
  await page.keyboard.up("y");
  await nothingAsked(page, 1);
  // Released and pressed again on the focused row: it acts.
  await page.keyboard.press("y");
  await expect.poll(async () => (await calls(page)).length).toBe(2);
  expect((await calls(page))[1]!.facts).toEqual([fact]);
});

/**
 * An engine whose auto-repeat does not mark the repeats (`repeat` false,
 * no keyup between them): the key is still known to be down.
 */
async function unmarkedDown(page: Page, key: string): Promise<void> {
  await page.evaluate((k) => {
    const at = document.activeElement ?? document.body;
    at.dispatchEvent(new KeyboardEvent("keydown", { key: k, code: `Key${k.toUpperCase()}`, bubbles: true, cancelable: true, repeat: false }));
  }, key);
}
async function unmarkedUp(page: Page, key: string): Promise<void> {
  await page.evaluate((k) => {
    const at = document.activeElement ?? document.body;
    at.dispatchEvent(new KeyboardEvent("keyup", { key: k, code: `Key${k.toUpperCase()}`, bubbles: true, cancelable: true }));
  }, key);
}

test("a held Y whose repeats the engine does not mark still decides once", async ({ page }) => {
  await openReview(page);
  await tabInto(page, rows(run(page, 41)).first());
  for (let i = 0; i < 5; i += 1) await unmarkedDown(page, "y");
  await expect(view(page).getByTestId("review-outcome")).toContainText("Accepted 1 Fact");
  await nothingAsked(page, 1);
  await unmarkedUp(page, "y");
});

test("a Y held from before the row had focus, with unmarked repeats, does nothing and says why", async ({ page }) => {
  await openReview(page);
  const tagging = run(page, 41);
  const first = rows(tagging).first();
  const fact = await factOf(first);
  // Down in the threshold field (typing: not the row's), then the focus
  // moves to the row with the key still held.
  await tagging.getByRole("spinbutton").focus();
  await unmarkedDown(page, "y");
  await tabInto(page, first);
  for (let i = 0; i < 3; i += 1) await unmarkedDown(page, "y");
  await expect(refusal(page)).toContainText("already held down");
  await nothingAsked(page);
  await unmarkedUp(page, "y");
  await unmarkedDown(page, "y");
  await expect.poll(async () => (await calls(page)).map((c) => c.facts)).toEqual([[fact]]);
});

test("a row that changed since it took focus is not decided, and the view says why", async ({ page }) => {
  await openReview(page);
  const first = rows(run(page, 41)).first();
  const fact = await factOf(first);
  // Focus by the pointer (it does not hold the list), then the pointer leaves.
  await first.getByRole("checkbox").click();
  await first.getByRole("checkbox").click();
  await page.mouse.move(2, 2);
  await hook(page, "__reviewChange", fact, 5);
  await expect(first.getByRole("cell").nth(4)).toHaveText("5");
  await expect(first.getByRole("checkbox")).toBeFocused();
  await page.keyboard.press("y");
  await expect(refusal(page)).toContainText("changed since");
  await nothingAsked(page);
  await page.keyboard.press("n");
  await expect(refusal(page)).toContainText("changed since");
  await nothingAsked(page);
  // Focus off the row and back: it is the row as it is now, and Y acts on
  // it with its new token.
  await page.keyboard.press("Shift+Tab");
  await expect(first.getByRole("checkbox")).not.toBeFocused();
  await page.keyboard.press("Tab");
  await expect(first.getByRole("checkbox")).toBeFocused();
  await page.keyboard.press("y");
  await expect.poll(async () => (await calls(page)).length).toBe(1);
  expect((await calls(page))[0]!.rows).toEqual([{ fact, token: `r${fact}.1` }]);
});

test("a row that moved since it took focus is not decided, and the view says why", async ({ page }) => {
  await openReview(page);
  const tagging = rows(run(page, 41));
  const second = view(page).locator(`[role="row"][data-fact="${await factOf(tagging.nth(1))}"]`);
  const above = await factOf(tagging.first());
  await second.getByRole("checkbox").click();
  await second.getByRole("checkbox").click();
  await page.mouse.move(2, 2);
  // Another client rejects the row above: the focused one moves up.
  await hook(page, "__reviewOtherReject", above);
  await expect(view(page).locator(`[role="row"][data-fact="${above}"]`)).toHaveCount(0);
  await expect(second.getByRole("checkbox")).toBeFocused();
  await page.keyboard.press("y");
  await expect(refusal(page)).toContainText("moved since");
  await nothingAsked(page);
});

test("Y and N do nothing from the threshold field or the command palette", async ({ page }) => {
  await openReview(page);
  const tagging = run(page, 41);
  await tagging.getByRole("spinbutton").focus();
  await page.keyboard.press("y");
  await page.keyboard.press("n");
  await nothingAsked(page);
  // The palette opened over a focused row: the keys are typed into it.
  await tabInto(page, rows(tagging).first());
  await page.keyboard.press("Control+Shift+P");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await expect(palette).toBeVisible();
  await page.keyboard.press("y");
  await page.keyboard.press("n");
  await expect(palette.getByRole("combobox").or(palette.getByRole("textbox")).first()).toHaveValue("yn");
  await nothingAsked(page);
});

test("Y says why it did nothing while Library Protection is on, and N still rejects", async ({ page }) => {
  await openReview(page);
  await hook(page, "__turnLibraryProtection", true);
  await expect(view(page).getByTestId("review-protected")).toBeVisible();
  const first = rows(run(page, 41)).first();
  await tabInto(page, first);
  await page.keyboard.press("y");
  await expect(refusal(page)).toContainText("Library Protection is on");
  await nothingAsked(page);
  await page.keyboard.press("n");
  await expect.poll(async () => (await calls(page)).map((c) => c.command)).toEqual(["review.reject"]);
});

test("N on a row never reaches the deck (it is the deck's Next memory cue)", async ({ page }) => {
  await openReview(page);
  const analysed = rows(run(page, 41)).nth(3);
  await analysed.getByRole("button", { name: /^Listen to / }).click();
  await expect(pauseButton(page)).toBeVisible();
  await pauseButton(page).click();
  await page.waitForTimeout(200);
  const at = await elapsedSeconds(page);
  // On the row's Listen button (from a checkbox the deck never takes a key).
  const target = rows(run(page, 38)).first();
  await tabInto(page, target);
  await page.keyboard.press("Tab");
  await expect(target.getByRole("button", { name: /^Listen to / })).toBeFocused();
  await page.keyboard.press("n");
  await expect(view(page).getByTestId("review-outcome")).toContainText("Rejected 1 Fact");
  await page.waitForTimeout(300);
  expect(await elapsedSeconds(page)).toBe(at);
});
