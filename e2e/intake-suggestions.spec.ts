/**
 * Proposed titles and artists in Intake (wbx-66). A proposal shows beside
 * its field, unlike the value set, and is accepted with a click or with Y
 * while it has the focus. Accepting is Review's own decision (`review.accept`
 * naming the row and its token): the value is written, the proposal leaves
 * Intake and Review; one decided in Review leaves Intake. Refusals (Library
 * Protection, a Stick-hosted Library, a proposal changed since it was shown)
 * are said, and nothing changes. A proposal arriving while a Run works
 * appears without reloading.
 *
 * Mock (`?proposals`, `src/ipc/mock-review.ts`): the first Intake track has
 * a proposed title "A Better Title" and artist "The Proposed Artist", the
 * second a proposed title "Second Proposal".
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { player, playButton, pauseButton, seedShell } from "./compactPlayer";

interface Call { command: string; facts: number[]; rows?: { fact: number; token: string }[] }

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const queue = (page: Page) => page.getByRole("listbox", { name: "Intake queue" });
const options = (page: Page) => queue(page).getByRole("option");
const panel = (page: Page) => page.getByRole("region", { name: "Track" });
const titleField = (page: Page) => panel(page).getByRole("textbox", { name: "Title" });
const artistField = (page: Page) => panel(page).getByRole("textbox", { name: "Artist" });
const proposals = (page: Page, field: "Title" | "Artist") => panel(page).getByRole("list", { name: `Proposed ${field}` });
const proposal = (page: Page, field: "Title" | "Artist", value: string): Locator =>
  proposals(page, field).getByRole("button", { name: new RegExp(`^Accept proposed ${field} “${value}”`) });
const hook = <T,>(page: Page, name: string, ...args: unknown[]) =>
  page.evaluate(([n, a]) => (window as unknown as Record<string, (...x: unknown[]) => T>)[n]!(...(a)), [name, args] as const);
const accepts = async (page: Page) => (await hook<Call[]>(page, "__reviewCalls")).filter((c) => c.command === "review.accept");

async function openIntake(page: Page, protect = false): Promise<void> {
  if (protect) {
    await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
      advanced: { protectLibrary: true }, view: { viewShell: true },
    })));
  } else {
    await seedShell(page);
  }
  await page.goto("/?proposals");
  await expect(page.getByTestId("shell")).toBeVisible({ timeout: 20_000 });
  await tab(page, "Intake").click();
  await expect(options(page).first()).toBeVisible();
}

/** Chooses the queue's row at `at`; answers its id and title. */
async function choose(page: Page, at = 0): Promise<{ id: string; title: string }> {
  const row = options(page).nth(at);
  const id = (await row.getAttribute("data-track"))!;
  const title = await row.locator("span").first().innerText();
  await row.click();
  await expect(titleField(page)).toHaveValue(title);
  return { id, title };
}

/** Keyboard focus onto a proposal: Tab from its field, as the DJ would. */
async function tabOnto(page: Page, field: Locator, target: Locator): Promise<void> {
  await field.focus();
  await page.keyboard.press("Tab");
  await expect(target).toBeFocused();
}

test("a proposed title shows beside its field, unlike the set value, and Y on it accepts it as Review's decision", async ({ page }) => {
  await openIntake(page);
  const { id, title } = await choose(page);
  const better = proposal(page, "Title", "A Better Title");
  await expect(better).toBeVisible();
  // The set value stays what the Library holds until the DJ accepts.
  await expect(titleField(page)).toHaveValue(title);
  await expect(better).toContainText("proposed");
  await expect(proposal(page, "Artist", "The Proposed Artist")).toBeVisible();

  await tabOnto(page, titleField(page), better);
  await page.keyboard.press("y");

  await expect(titleField(page)).toHaveValue("A Better Title");
  await expect(queue(page).locator(`[data-track="${id}"] span`).first()).toHaveText("A Better Title");
  await expect(proposals(page, "Title")).toHaveCount(0);
  // The artist still waits.
  await expect(proposal(page, "Artist", "The Proposed Artist")).toBeVisible();
  const calls = await accepts(page);
  expect(calls).toHaveLength(1);
  expect(calls[0]!.rows).toHaveLength(1);
  expect(calls[0]!.rows![0]!.token).not.toBe("");
});

test("a click accepts a proposed artist and it leaves Review; one rejected in Review leaves Intake", async ({ page }) => {
  await openIntake(page);
  await choose(page);
  await proposal(page, "Artist", "The Proposed Artist").click();
  await expect(artistField(page)).toHaveValue("The Proposed Artist");
  await expect(proposals(page, "Artist")).toHaveCount(0);

  await tab(page, "Review").click();
  const review = page.getByTestId("review-view");
  await expect(review.getByRole("button", { name: /^Reject Title Second Proposal/ })).toBeVisible();
  await expect(review.getByRole("button", { name: /^Reject Artist The Proposed Artist/ })).toHaveCount(0);
  await review.getByRole("button", { name: /^Reject Title Second Proposal/ }).click();
  await expect(review.getByRole("button", { name: /^Reject Title Second Proposal/ })).toHaveCount(0);

  await tab(page, "Intake").click();
  await choose(page, 1);
  await expect(proposals(page, "Title")).toHaveCount(0);
  expect(await accepts(page)).toHaveLength(1);
});

test("a proposal recorded while a Run works appears without reloading", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page, 1);
  await expect(proposal(page, "Title", "Second Proposal")).toBeVisible();
  await expect(proposals(page, "Artist")).toHaveCount(0);
  await hook(page, "__reviewPropose", id, "artist", "Live Artist");
  await expect(proposal(page, "Artist", "Live Artist")).toBeVisible();
});

test("under Library Protection an accept is refused, said, and nothing changes", async ({ page }) => {
  await openIntake(page, true);
  const { title } = await choose(page);
  const better = proposal(page, "Title", "A Better Title");
  await better.click();
  await expect(panel(page).getByTestId("intake-edit-failed")).toContainText("Library Protection");
  await tabOnto(page, titleField(page), better);
  await page.keyboard.press("y");
  await expect(panel(page).getByTestId("intake-suggestion-key-refused")).toContainText("Library Protection is on");
  await page.waitForTimeout(300);
  expect(await accepts(page)).toHaveLength(0);
  await expect(titleField(page)).toHaveValue(title);
  await expect(better).toBeVisible();
});

test("a Stick-hosted Library refuses the accept; the refusal is shown and the proposal stays", async ({ page }) => {
  await openIntake(page);
  const { title } = await choose(page);
  await hook(page, "__reviewNext", "sidecar_on_stick");
  await proposal(page, "Title", "A Better Title").click();
  await expect(panel(page).getByTestId("intake-edit-failed")).toContainText("sidecar_on_stick");
  await expect(titleField(page)).toHaveValue(title);
  await expect(proposal(page, "Title", "A Better Title")).toBeVisible();
});

test("a proposal changed since it was shown is refused with the reason, then shown as it is now", async ({ page }) => {
  await openIntake(page);
  const { title } = await choose(page);
  const fact = Number(await proposal(page, "Title", "A Better Title").getAttribute("data-suggestion"));
  await hook(page, "__holdIntakeReads");
  await hook(page, "__reviewChange", fact, "Changed Meanwhile");
  await proposal(page, "Title", "A Better Title").click();
  await expect(panel(page).getByTestId("intake-edit-failed")).toContainText("changed since");
  await expect(titleField(page)).toHaveValue(title);
  await hook(page, "__releaseIntakeReads");
  await expect(proposal(page, "Title", "Changed Meanwhile")).toBeVisible();
});

test("Y: typed in the title field it is text; held on a proposal it accepts once; it never reaches the deck", async ({ page }) => {
  await openIntake(page);
  const { title } = await choose(page);
  await page.evaluate(() => {
    const w = window as unknown as { __windowYs: number };
    w.__windowYs = 0;
    window.addEventListener("keydown", (event) => { if (event.key.toLowerCase() === "y") w.__windowYs += 1; });
  });
  // From the field: typing, never a decision.
  await titleField(page).focus();
  await page.keyboard.press("End");
  await page.keyboard.press("y");
  await expect(titleField(page)).toHaveValue(`${title}y`);
  await page.keyboard.press("Escape");
  await expect(titleField(page)).toHaveValue(title);
  expect(await accepts(page)).toHaveLength(0);

  // Held on the artist proposal: one decision.
  const artist = proposal(page, "Artist", "The Proposed Artist");
  await tabOnto(page, artistField(page), artist);
  await page.keyboard.down("y");
  for (let i = 0; i < 5; i += 1) await page.keyboard.down("y");
  await page.keyboard.up("y");
  await expect(artistField(page)).toHaveValue("The Proposed Artist");
  await page.waitForTimeout(300);
  expect(await accepts(page)).toHaveLength(1);
  // A Y on a proposal stopped there; the field's own Ys are the field's.
  expect(await page.evaluate(() => (window as unknown as { __windowYs: number }).__windowYs)).toBe(0);
});

test("a Y held from before a proposal had focus does nothing and says why; a fresh press then accepts", async ({ page }) => {
  await openIntake(page);
  await choose(page);
  const better = proposal(page, "Title", "A Better Title");
  const artist = proposal(page, "Artist", "The Proposed Artist");
  await tabOnto(page, artistField(page), artist);
  await page.keyboard.down("y");
  await expect.poll(async () => (await accepts(page)).length).toBe(1);
  await better.focus();
  await page.keyboard.down("y");
  await page.keyboard.up("y");
  await expect(panel(page).getByTestId("intake-suggestion-key-refused")).toContainText("already held down");
  expect(await accepts(page)).toHaveLength(1);
  await page.keyboard.press("y");
  await expect.poll(async () => (await accepts(page)).length).toBe(2);
  await expect(titleField(page)).toHaveValue("A Better Title");
});

test("Space and Enter on a proposal accept it and never play or pause the deck", async ({ page }) => {
  await openIntake(page);
  await choose(page);
  await expect(playButton(page)).toBeVisible();
  const better = proposal(page, "Title", "A Better Title");
  await tabOnto(page, titleField(page), better);
  await page.keyboard.press(" ");
  await expect(titleField(page)).toHaveValue("A Better Title");
  await expect(playButton(page)).toBeVisible();
  await expect(pauseButton(page)).toHaveCount(0);
  await expect(player(page)).toBeVisible();
});

test("an Enter held from the title field onto a proposal does nothing and says why; a fresh Enter accepts once", async ({ page }) => {
  await openIntake(page);
  const { title } = await choose(page);
  const better = proposal(page, "Title", "A Better Title");
  await titleField(page).focus();
  await page.keyboard.down("Enter");
  await page.keyboard.press("Tab");
  await expect(better).toBeFocused();
  // The held Enter's repeats on the proposal.
  for (let i = 0; i < 3; i += 1) await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await expect(panel(page).getByTestId("intake-suggestion-key-refused")).toContainText("Enter did nothing: it was already held down");
  await page.waitForTimeout(300);
  expect(await accepts(page)).toHaveLength(0);
  await expect(titleField(page)).toHaveValue(title);
  // Released and pressed again: one accept, never two.
  await page.keyboard.press("Enter");
  await expect(titleField(page)).toHaveValue("A Better Title");
  await page.waitForTimeout(300);
  expect(await accepts(page)).toHaveLength(1);
});

test("Enter or Space on a proposal that changed since it took focus is not accepted, and says why", async ({ page }) => {
  await openIntake(page);
  const { title } = await choose(page);
  const better = proposal(page, "Title", "A Better Title");
  const fact = Number(await better.getAttribute("data-suggestion"));
  await tabOnto(page, titleField(page), better);
  const reads = await hook<{ queue: number }>(page, "__intakeReads");
  // Same text, a new row token: changed under the DJ's focus.
  await hook(page, "__reviewChange", fact, "A Better Title");
  await expect.poll(async () => (await hook<{ queue: number }>(page, "__intakeReads")).queue).toBeGreaterThan(reads.queue);
  await page.waitForTimeout(200);
  await expect(better).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(panel(page).getByTestId("intake-suggestion-key-refused")).toContainText("Enter did nothing: this proposal changed since it took focus");
  await page.keyboard.press(" ");
  await expect(panel(page).getByTestId("intake-suggestion-key-refused")).toContainText("Space did nothing: this proposal changed since it took focus");
  await page.waitForTimeout(300);
  expect(await accepts(page)).toHaveLength(0);
  await expect(titleField(page)).toHaveValue(title);
});

test("Review's confidence bulk accept never takes a proposed title or artist; its count says what it takes", async ({ page }) => {
  await openIntake(page);
  await tab(page, "Review").click();
  const review = page.getByTestId("review-view");
  const tagging = review.locator('[data-run="41"]');
  const bulk = tagging.getByTestId("review-bulk-accept");
  // The Run's Energy rows at or above 0.80 (5); "A Better Title" (0.82) is not counted.
  await expect(tagging.getByRole("button", { name: /^Accept Title A Better Title/ })).toBeVisible();
  await expect(bulk).toHaveText("Accept ≥ 0.80 · 5");
  await bulk.click();
  await expect.poll(async () => (await accepts(page)).length).toBe(1);
  const [call] = await accepts(page);
  expect(call!.facts).toHaveLength(5);
  // Still waiting, by its own row.
  await expect(tagging.getByRole("button", { name: /^Accept Title A Better Title/ })).toBeVisible();
  await tab(page, "Intake").click();
  await choose(page);
  await expect(proposal(page, "Title", "A Better Title")).toBeVisible();
});
