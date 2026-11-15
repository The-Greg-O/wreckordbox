/**
 * Proposed tags in Intake (wbx-45.26). A proposal shows as a tag that looks
 * unlike a set one, and is accepted with a click or with the same guarded
 * keys as a proposed title. Accepting is Review's own decision: the MyTag
 * is set, and the row leaves Intake and Review. One decided in Review
 * leaves Intake. Library Protection and a Stick-hosted Library refuse it,
 * said, with nothing written. A proposal recorded while a Run works
 * appears without reloading.
 *
 * Mock (`?tagSchema=five&tagProposals`): the first Intake track has a
 * proposed Situation "Warm-up" (dimension "setting", confidence 0.84).
 * The fixture's first Intake track already carries Peak.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { player, playButton, pauseButton, seedShell } from "./compactPlayer";
import { announce, LATTE, rgb } from "./lightPalettes";

interface Call { command: string; facts: number[]; rows?: { fact: number; token: string }[] }

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const options = (page: Page) => page.getByRole("listbox", { name: "Intake queue" }).getByRole("option");
const panel = (page: Page) => page.getByRole("region", { name: "Track" });
const tags = (page: Page) => page.getByRole("region", { name: "Tags" });
const situation = (page: Page) => tags(page).getByRole("group", { name: "Situation", exact: true });
const proposed = (page: Page, name: string): Locator =>
  situation(page).getByRole("button", { name: new RegExp(`^Accept proposed Situation “${name}”`) });
const setTag = (page: Page, name: string) => situation(page).getByRole("button", { name, exact: true });
const titleField = (page: Page) => panel(page).getByRole("textbox", { name: "Title" });
const hook = <T,>(page: Page, name: string, ...args: unknown[]) =>
  page.evaluate(([n, a]) => (window as unknown as Record<string, (...x: unknown[]) => T>)[n]!(...(a)), [name, args] as const);
const accepts = async (page: Page) => (await hook<Call[]>(page, "__reviewCalls")).filter((c) => c.command === "review.accept");
const myTags = (page: Page, id: string) => hook<string[]>(page, "__myTagsOf", id);

async function openIntake(page: Page, query = "?tagSchema=five&tagProposals", protect = false): Promise<void> {
  if (protect) {
    await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
      advanced: { protectLibrary: true }, view: { viewShell: true },
    })));
  } else {
    await seedShell(page);
  }
  await page.goto(`/${query}`);
  await expect(page.getByTestId("shell")).toBeVisible({ timeout: 20_000 });
  await tab(page, "Intake").click();
  await expect(options(page).first()).toBeVisible();
}

async function choose(page: Page, at = 0): Promise<{ id: string; title: string }> {
  const row = options(page).nth(at);
  const id = (await row.getAttribute("data-track"))!;
  const title = await row.locator("span").first().innerText();
  await row.click();
  await expect(tags(page)).toHaveAttribute("data-track", id);
  await expect(situation(page)).toBeVisible();
  return { id, title };
}

test("a proposed tag is dashed and marked proposed, and a click accepts it as Review's decision", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  const warm = proposed(page, "Warm-up");
  await expect(warm).toBeVisible();
  await expect(warm).toContainText("proposed");
  await expect(warm).toHaveAttribute("aria-pressed", "false");
  await expect(warm).toHaveAttribute("data-state", "proposed");
  const look = await situation(page).evaluate((group) => {
    const suggested = group.querySelector("[data-state='proposed']");
    const plain = group.querySelector("button:not([data-state='proposed'])");
    if (!(suggested instanceof HTMLElement) || !(plain instanceof HTMLElement)) return null;
    const chip = getComputedStyle(suggested);
    const other = getComputedStyle(plain);
    return { dashed: chip.borderTopStyle, plain: other.borderTopStyle, color: chip.color, plainColor: other.color };
  });
  expect(look?.dashed).toBe("dashed");
  expect(look?.plain).not.toBe("dashed");
  expect(look?.color).not.toBe(look?.plainColor);
  expect(await myTags(page, id)).not.toContain("Warm-up");

  await warm.click();

  await expect(setTag(page, "Warm-up")).toHaveAttribute("aria-pressed", "true");
  await expect(proposed(page, "Warm-up")).toHaveCount(0);
  expect(await myTags(page, id)).toContain("Warm-up");
  const calls = await accepts(page);
  expect(calls).toHaveLength(1);
  expect(calls[0]!.rows).toHaveLength(1);
  expect(calls[0]!.rows![0]!.token).not.toBe("");

  await tab(page, "Review").click();
  const review = page.getByTestId("review-view");
  await expect(review.locator('[data-run="41"]').getByTestId("review-bulk-accept")).toBeVisible();
  await expect(review.getByRole("button", { name: /^Accept Tag Situation: Warm-up/ })).toHaveCount(0);
});

test("a tag decided in Review leaves Intake, and one recorded live appears without reloading", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  await expect(proposed(page, "Warm-up")).toBeVisible();

  await tab(page, "Review").click();
  const review = page.getByTestId("review-view");
  await expect(review.locator('[data-run="41"]').getByTestId("review-bulk-accept")).toBeVisible();
  await review.getByRole("button", { name: /^Reject Tag Situation: Warm-up/ }).click();
  await expect(review.getByRole("button", { name: /Tag Situation: Warm-up/ })).toHaveCount(0);

  await tab(page, "Intake").click();
  await choose(page);
  await expect(proposed(page, "Warm-up")).toHaveCount(0);
  expect(await myTags(page, id)).not.toContain("Warm-up");

  const second = await choose(page, 1);
  await expect(proposed(page, "Peak")).toHaveCount(0);
  await hook(page, "__reviewProposeTag", second.id, "setting", ["peak"], "Peak", "Situation");
  await expect(proposed(page, "Peak")).toBeVisible();
  await expect(proposed(page, "Peak")).toHaveAttribute("data-state", "proposed");
});

test("Y, Space and Enter accept the focused proposal once and never reach the deck or a text field", async ({ page }) => {
  await openIntake(page);
  const { id, title } = await choose(page);
  await page.evaluate(() => {
    const w = window as unknown as { __windowYs: number };
    w.__windowYs = 0;
    window.addEventListener("keydown", (event) => { if (event.key.toLowerCase() === "y") w.__windowYs += 1; });
  });
  await expect(playButton(page)).toBeVisible();

  await titleField(page).focus();
  await page.keyboard.press("End");
  await page.keyboard.press("y");
  await expect(titleField(page)).toHaveValue(`${title}y`);
  await page.keyboard.press("Escape");
  await expect(titleField(page)).toHaveValue(title);
  expect(await accepts(page)).toHaveLength(0);

  const warm = proposed(page, "Warm-up");
  await titleField(page).focus();
  await page.keyboard.down("y");
  await warm.focus();
  await page.keyboard.down("y");
  await page.keyboard.up("y");
  await expect(panel(page).getByTestId("intake-suggestion-key-refused")).toContainText("already held down");
  expect(await accepts(page)).toHaveLength(0);
  await expect(warm).toBeVisible();

  await page.keyboard.down("y");
  for (let i = 0; i < 4; i += 1) await page.keyboard.down("y");
  await page.keyboard.up("y");
  await expect(setTag(page, "Warm-up")).toHaveAttribute("aria-pressed", "true");
  await page.waitForTimeout(300);
  expect(await accepts(page)).toHaveLength(1);
  expect(await myTags(page, id)).toContain("Warm-up");
  await expect(playButton(page)).toBeVisible();
  await expect(pauseButton(page)).toHaveCount(0);
  await expect(player(page)).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __windowYs: number }).__windowYs)).toBe(0);
});

test("Space on a proposal that changed since it took focus does nothing and says why", async ({ page }) => {
  await openIntake(page);
  await choose(page);
  const warm = proposed(page, "Warm-up");
  const fact = Number(await warm.getAttribute("data-suggestion"));
  await warm.focus();
  const reads = await hook<{ queue: number }>(page, "__intakeReads");
  await hook(page, "__reviewChange", fact, { dimension: "setting", terms: ["warm-up"], label: "Warm-up", dimension_label: "Situation" });
  await expect.poll(async () => (await hook<{ queue: number }>(page, "__intakeReads")).queue).toBeGreaterThan(reads.queue);
  await expect(warm).toBeFocused();
  await page.keyboard.press(" ");
  await expect(panel(page).getByTestId("intake-suggestion-key-refused")).toContainText("Space did nothing: this proposal changed since it took focus");
  await page.waitForTimeout(300);
  expect(await accepts(page)).toHaveLength(0);
  await expect(playButton(page)).toBeVisible();
  await expect(proposed(page, "Warm-up")).toBeVisible();
});

test("under Library Protection an accept is refused, said, and the tag stays unset", async ({ page }) => {
  await openIntake(page, "?tagSchema=five&tagProposals", true);
  const { id } = await choose(page);
  const warm = proposed(page, "Warm-up");
  await warm.click();
  await expect(panel(page).getByTestId("intake-edit-failed")).toContainText("Library Protection");
  await warm.focus();
  await page.keyboard.press("y");
  await expect(panel(page).getByTestId("intake-suggestion-key-refused")).toContainText("Library Protection is on");
  await page.waitForTimeout(300);
  expect(await accepts(page)).toHaveLength(0);
  expect(await myTags(page, id)).not.toContain("Warm-up");
  await expect(warm).toBeVisible();
});

test("a Stick-hosted Library refuses the accept; the refusal is shown and the proposal stays", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  await hook(page, "__reviewNext", "sidecar_on_stick");
  await proposed(page, "Warm-up").click();
  await expect(panel(page).getByTestId("intake-edit-failed")).toContainText("sidecar_on_stick");
  await expect(proposed(page, "Warm-up")).toBeVisible();
  expect(await myTags(page, id)).not.toContain("Warm-up");
});

test("Review's confidence bulk accept takes a proposed tag and still never a title", async ({ page }) => {
  await openIntake(page, "?proposals&tagSchema=five&tagProposals");
  await tab(page, "Review").click();
  const review = page.getByTestId("review-view");
  const tagging = review.locator('[data-run="41"]');
  const bulk = tagging.getByTestId("review-bulk-accept");
  await expect(tagging.getByRole("button", { name: /^Accept Title A Better Title/ })).toBeVisible();
  await expect(tagging.getByRole("button", { name: /^Accept Tag Situation: Warm-up/ })).toBeVisible();
  await expect(bulk).toHaveText("Accept ≥ 0.80 · 6");
  await bulk.click();
  await expect.poll(async () => (await accepts(page)).length).toBe(1);
  const [call] = await accepts(page);
  expect(call!.facts).toHaveLength(6);
  await expect(tagging.getByRole("button", { name: /^Accept Title A Better Title/ })).toBeVisible();
  await expect(tagging.getByRole("button", { name: /^Accept Tag Situation: Warm-up/ })).toHaveCount(0);
});

test("under a light theme the proposed tag's name is the theme text, not the yellow", async ({ page }) => {
  await openIntake(page);
  await choose(page);
  const warm = proposed(page, "Warm-up");
  await announce(page, { roles: LATTE, font: null });
  await expect(page.locator("html")).toHaveAttribute("data-theme-mode", "light");
  const colors = await warm.evaluate((el) => {
    const chip = getComputedStyle(el);
    const mark = el.querySelector("span");
    return { name: chip.color, mark: mark ? getComputedStyle(mark).color : "", border: chip.borderTopStyle };
  });
  expect(colors.border).toBe("dashed");
  expect(colors.name).toBe(rgb(LATTE.text));
  expect(colors.mark).not.toBe(rgb("#e0af68"));
  expect(colors.name).not.toBe(rgb("#e0af68"));
});
