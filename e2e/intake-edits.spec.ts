/**
 * Intake edits (wbx-63): the track panel beside the Intake queue. Title and
 * artist are edited in place and saved on commit; Energy is set 1 to 5 by
 * click or Shift with the digit; the Prep colour is any of the DJ's labels;
 * "Mark Clean and next" (Ctrl+Enter) sets Clean and loads the next queued
 * track as one undoable step; P marks Problem and moves on; ] skips. What
 * is still empty is shown and blocks nothing; an edit made elsewhere (the
 * CLI, an agent) shows at once.
 *
 * Mock backend (`src/ipc/mock-intake.ts`): Clean is Green (5), Problem Red
 * (2); Pink, Orange and Blue are named Sample, Buy and Delete.
 * `__prepColourOf(id)`, `__prepWrites()`, `__ratingOf(id)` read what the
 * mock Library holds; `__outsideEdit(id, field, value)` is someone else's
 * edit.
 */
import { expect, test, type Page } from "@playwright/test";
import { player, seedShell } from "./compactPlayer";

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const queue = (page: Page) => page.getByRole("listbox", { name: "Intake queue" });
const options = (page: Page) => queue(page).getByRole("option");
const panel = (page: Page) => page.getByRole("region", { name: "Track" });
const loadedTitle = (page: Page) => player(page).getByTestId("compact-player-title");
const energy = (page: Page) => panel(page).getByRole("radiogroup", { name: "Energy" });
const prep = (page: Page) => panel(page).getByRole("radiogroup", { name: "Prep colour" });
const playing = (page: Page) => player(page).getByRole("button", { name: "Pause", exact: true });

const prepColourOf = (page: Page, id: string) =>
  page.evaluate((track) => (window as unknown as { __prepColourOf: (id: string) => number }).__prepColourOf(track), id);
const prepWrites = (page: Page) =>
  page.evaluate(() => (window as unknown as { __prepWrites: () => { id: string; colour: number | null }[] }).__prepWrites());
const ratingOf = (page: Page, id: string) =>
  page.evaluate((track) => (window as unknown as { __ratingOf: (id: string) => number }).__ratingOf(track), id);
const outsideEdit = (page: Page, id: string, field: "title" | "artist" | "energy", value: string) =>
  page.evaluate(([track, which, to]) => (window as unknown as {
    __outsideEdit: (id: string, field: string, value: string) => Promise<unknown>;
  }).__outsideEdit(track!, which!, to!), [id, field, value]);

async function openIntake(page: Page, protect = false): Promise<void> {
  if (protect) {
    await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
      advanced: { protectLibrary: true }, view: { viewShell: true },
    })));
  } else {
    await seedShell(page);
  }
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  await tab(page, "Intake").click();
  await expect(options(page).first()).toBeVisible();
}

/** Chooses the queue's row at `at` and waits for the player to hold it; answers its id and title. */
async function choose(page: Page, at = 0): Promise<{ id: string; title: string }> {
  const row = options(page).nth(at);
  const id = (await row.getAttribute("data-track"))!;
  const title = await row.locator("span").first().innerText();
  await row.click();
  await expect(loadedTitle(page)).toHaveText(title);
  await expect(panel(page).getByRole("textbox", { name: "Title" })).toHaveValue(title);
  return { id, title };
}

const rowOf = (page: Page, id: string) => queue(page).locator(`[data-track="${id}"]`);

test("the title and artist are edited in place and saved on commit; Escape keeps what was there", async ({ page }) => {
  await openIntake(page);
  const { id, title } = await choose(page);
  const field = panel(page).getByRole("textbox", { name: "Title" });

  await field.fill("Fixed Title (Original Mix)");
  await field.press("Escape");
  await expect(field).toHaveValue(title);
  await expect(rowOf(page, id).locator("span").first()).toHaveText(title);

  await field.fill("Fixed Title (Original Mix)");
  await field.press("Enter");
  // Saved: the queue (the service's read) and the player say it.
  await expect(rowOf(page, id).locator("span").first()).toHaveText("Fixed Title (Original Mix)");
  await expect(field).toHaveValue("Fixed Title (Original Mix)");

  // Leaving the field commits too.
  const artist = panel(page).getByRole("textbox", { name: "Artist" });
  await artist.fill("Someone Else");
  await panel(page).getByRole("radiogroup", { name: "Energy" }).getByRole("radio", { name: "Energy 1" }).focus();
  await expect(rowOf(page, id)).toContainText("Someone Else");
});

test("Energy is set by click and by Shift with the digit", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  // A level the track does not have (clicking the one it has clears it).
  const level = (await ratingOf(page, id)) === 3 ? 2 : 3;

  await energy(page).getByRole("radio", { name: `Energy ${level}` }).click();
  await expect.poll(() => ratingOf(page, id)).toBe(level);
  await expect(energy(page).getByRole("radio", { name: `Energy ${level}` })).toHaveAttribute("aria-checked", "true");
  // Clicking the set level again clears it.
  await energy(page).getByRole("radio", { name: `Energy ${level}` }).click();
  await expect.poll(() => ratingOf(page, id)).toBe(0);

  // From the queue, with no field focused: Shift+5.
  await options(page).first().focus();
  await page.keyboard.press("Shift+Digit5");
  await expect.poll(() => ratingOf(page, id)).toBe(5);
  await expect(energy(page).getByRole("radio", { name: "Energy 5" })).toHaveAttribute("aria-checked", "true");
  // The view stayed on Intake (Shift+digit is not a view key).
  await expect(tab(page, "Intake")).toHaveAttribute("aria-selected", "true");
});

test("the Prep colour can be any of the DJ's labels", async ({ page }) => {
  await openIntake(page);
  const radios = prep(page).getByRole("radio");
  await choose(page);
  await expect(radios).toHaveText(["Sample", "Problem", "Buy", "Yellow", "Clean", "Aqua", "Delete", "Purple"]);

  const { id } = await choose(page, 1);
  await prep(page).getByRole("radio", { name: "Delete" }).click();
  await expect.poll(() => prepColourOf(page, id)).toBe(7);
  // Coloured, it has left Intake.
  await expect(rowOf(page, id)).toHaveCount(0);
});

/** Ctrl+Z with nothing focused: the window's Undo. */
async function undo(page: Page): Promise<void> {
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press("Control+z");
}

for (const how of ["Mark Clean and next", "P"] as const) {
  test(`${how}: one Undo takes the whole action back, the track queued, selected and loaded again`, async ({ page }) => {
    await openIntake(page);
    const second = await options(page).nth(1).locator("span").first().innerText();
    const { id, title } = await choose(page);

    if (how === "P") {
      await options(page).first().focus();
      await page.keyboard.press("p");
    } else {
      await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
    }

    await expect.poll(() => prepColourOf(page, id)).toBe(how === "P" ? 2 : 5);
    await expect(loadedTitle(page)).toHaveText(second);
    await expect(rowOf(page, id)).toHaveCount(0);
    await expect(options(page).first()).toHaveAttribute("aria-selected", "true");
    await expect(panel(page).getByRole("textbox", { name: "Title" })).toHaveValue(second);

    await undo(page);
    await expect.poll(() => prepColourOf(page, id)).toBe(0);
    await expect(rowOf(page, id)).toHaveAttribute("aria-selected", "true");
    await expect(loadedTitle(page)).toHaveText(title);
    await expect(panel(page).getByRole("textbox", { name: "Title" })).toHaveValue(title);
  });
}

test("Undo of Mark Clean and next across a page boundary goes back to the page and the track", async ({ page }) => {
  await openIntake(page);
  // The last track of the first page: the next one is on the second page.
  const { id, title } = await choose(page, 199);
  await expect(page.getByTestId("intake-page")).toHaveText(/^1–200 of /);

  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
  await expect.poll(() => prepColourOf(page, id)).toBe(5);
  await expect(page.getByTestId("intake-page")).not.toHaveText(/^1–200 of /);
  await expect(loadedTitle(page)).not.toHaveText(title);

  await undo(page);
  await expect(page.getByTestId("intake-page")).toHaveText(/^1–200 of /);
  await expect(rowOf(page, id)).toHaveAttribute("aria-selected", "true");
  await expect(loadedTitle(page)).toHaveText(title);
});

test("Ctrl+Enter marks Clean and next, also straight from the title field after an edit", async ({ page }) => {
  await openIntake(page);
  const third = await options(page).nth(2).locator("span").first().innerText();
  const { id: first } = await choose(page);

  await options(page).first().focus();
  await page.keyboard.press("Control+Enter");
  await expect.poll(() => prepColourOf(page, first)).toBe(5);
  const second = options(page).first();
  await expect(second).toHaveAttribute("aria-selected", "true");
  const secondId = (await second.getAttribute("data-track"))!;

  // Typing a fix, then Ctrl+Enter in the field: the fix is saved, then marked.
  const field = panel(page).getByRole("textbox", { name: "Title" });
  await field.fill("Typed Then Marked");
  await field.press("Control+Enter");
  await expect.poll(() => prepColourOf(page, secondId)).toBe(5);
  await expect(loadedTitle(page)).toHaveText(third);
  const writes = await prepWrites(page);
  expect(writes.map((write) => write.id)).toEqual([first, secondId]);
});

test("P marks Problem and moves on; ] skips without changing the track", async ({ page }) => {
  await openIntake(page);
  const { id: first } = await choose(page);
  const nextTitle = await options(page).nth(1).locator("span").first().innerText();

  await options(page).first().focus();
  await page.keyboard.press("]");
  await expect(loadedTitle(page)).toHaveText(nextTitle);
  expect(await prepColourOf(page, first)).toBe(0);
  expect(await prepWrites(page)).toEqual([]);
  await expect(rowOf(page, first)).toHaveCount(1);

  const skippedTo = (await options(page).nth(1).getAttribute("data-track"))!;
  const after = await options(page).nth(2).locator("span").first().innerText();
  await page.keyboard.press("p");
  await expect.poll(() => prepColourOf(page, skippedTo)).toBe(2);
  await expect(loadedTitle(page)).toHaveText(after);
  await expect(rowOf(page, skippedTo)).toHaveCount(0);
});

test("what is still empty is shown and blocks nothing", async ({ page }) => {
  await openIntake(page);
  // A track that needs everything is marked Clean all the same.
  const needy = options(page).filter({ has: page.getByTestId("intake-needs").filter({ hasText: "needs everything" }) }).first();
  const id = (await needy.getAttribute("data-track"))!;
  await needy.click();
  await expect(panel(page).getByTestId("intake-panel-needs")).toHaveText("needs everything");
  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
  await expect.poll(() => prepColourOf(page, id)).toBe(5);
});

test("an edit made through the CLI to the track on screen appears without reloading", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);

  await outsideEdit(page, id, "title", "Renamed By An Agent");
  await outsideEdit(page, id, "artist", "Agent Artist");
  await outsideEdit(page, id, "energy", "4");

  await expect(panel(page).getByRole("textbox", { name: "Title" })).toHaveValue("Renamed By An Agent");
  await expect(panel(page).getByRole("textbox", { name: "Artist" })).toHaveValue("Agent Artist");
  await expect(energy(page).getByRole("radio", { name: "Energy 4" })).toHaveAttribute("aria-checked", "true");
});

test("with Library Protection on, Mark Clean and next is refused, said, and the track stays", async ({ page }) => {
  await openIntake(page, true);
  const { id, title } = await choose(page);

  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();

  await expect(page.getByTestId("shell-status")).toContainText("Editing is locked by Library Protection");
  await expect(panel(page).getByTestId("intake-edit-failed")).toContainText("Library Protection");
  expect(await prepWrites(page)).toEqual([]);
  expect(await prepColourOf(page, id)).toBe(0);
  await expect(loadedTitle(page)).toHaveText(title);
  await expect(rowOf(page, id)).toHaveAttribute("aria-selected", "true");
});

test("Space and Enter on the panel's controls never reach the deck", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();

  const level = (await ratingOf(page, id)) === 2 ? 4 : 2;
  await energy(page).getByRole("radio", { name: `Energy ${level}` }).focus();
  await page.keyboard.press("Space");
  await expect.poll(() => ratingOf(page, id)).toBe(level);
  await expect(playing(page)).toHaveCount(0);

  // Typing p and ] in a field is text, not Problem or skip.
  const field = panel(page).getByRole("textbox", { name: "Artist" });
  await field.fill("");
  await field.type("p]");
  await expect(field).toHaveValue("p]");
  expect(await prepWrites(page)).toEqual([]);
  await expect(playing(page)).toHaveCount(0);
});

const hold = (page: Page, hook: string) => page.evaluate((name) => (window as unknown as Record<string, () => void>)[name]!(), hook);
const fieldWrites = (page: Page) =>
  page.evaluate(() => (window as unknown as { __fieldWrites: () => { id: string; field: string; value: string }[] }).__fieldWrites());

test("title commits one after another without leaving the field each save, and Escape returns to the latest saved", async ({ page }) => {
  await openIntake(page);
  const { id, title } = await choose(page);
  const field = panel(page).getByRole("textbox", { name: "Title" });

  await field.fill("Second Title");
  await field.press("Enter");
  await expect(rowOf(page, id).locator("span").first()).toHaveText("Second Title");
  // Back to the first title, still in the field: saved too.
  await field.fill(title);
  await field.press("Enter");
  await expect(rowOf(page, id).locator("span").first()).toHaveText(title);
  expect((await fieldWrites(page)).map((write) => write.value)).toEqual(["Second Title", title]);

  // Escape goes back to the latest saved value, not the one the field started with.
  await field.fill("Third Title");
  await field.press("Enter");
  await expect(rowOf(page, id).locator("span").first()).toHaveText("Third Title");
  await field.fill("Not Saved");
  await field.press("Escape");
  await expect(field).toHaveValue("Third Title");
});

test("an edit from elsewhere during a typed draft keeps the draft, and Escape then shows the new value", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  const field = panel(page).getByRole("textbox", { name: "Title" });

  await field.fill("My Draft");
  await outsideEdit(page, id, "title", "Agent Title");
  await expect(rowOf(page, id).locator("span").first()).toHaveText("Agent Title");
  await expect(field).toHaveValue("My Draft");
  await field.press("Escape");
  await expect(field).toHaveValue("Agent Title");
  // The agent's value is now the baseline: committing it again writes nothing.
  const before = (await fieldWrites(page)).length;
  await field.press("Enter");
  await outsideEdit(page, id, "artist", "Someone");
  await expect(rowOf(page, id)).toContainText("Someone");
  expect((await fieldWrites(page)).length).toBe(before);

  // With nothing typed, a value from elsewhere replaces what the focused field shows.
  await field.focus();
  await outsideEdit(page, id, "title", "Agent Again");
  await expect(field).toHaveValue("Agent Again");
});

test("a held Ctrl+Enter in the title field saves once, then marks once, only after the save", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  const field = panel(page).getByRole("textbox", { name: "Title" });
  await hold(page, "__holdFieldWrites");

  await field.fill("Slow Save");
  await page.keyboard.down("Control");
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await page.keyboard.up("Control");
  await page.waitForTimeout(300);
  expect(await prepWrites(page)).toEqual([]);

  await hold(page, "__releaseFieldWrites");
  await expect.poll(() => prepColourOf(page, id)).toBe(5);
  await page.waitForTimeout(300);
  expect(await prepWrites(page)).toEqual([{ id, colour: 5 }]);
  expect((await fieldWrites(page)).filter((write) => write.value === "Slow Save")).toHaveLength(1);
});

test("a save that is refused is not followed by the mark", async ({ page }) => {
  await openIntake(page, true);
  const { id } = await choose(page);
  const field = panel(page).getByRole("textbox", { name: "Title" });
  await field.fill("Refused");
  await field.press("Control+Enter");
  await expect(panel(page).getByTestId("intake-edit-failed")).toContainText("Library Protection");
  await page.waitForTimeout(300);
  expect(await prepWrites(page)).toEqual([]);
  expect(await prepColourOf(page, id)).toBe(0);
});

test("a held key never edits twice: P and Shift+digit act once", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  await options(page).first().focus();
  const ratingsBefore = await page.evaluate(() => (window as unknown as { __ratingWrites: unknown[] }).__ratingWrites.length);

  await page.keyboard.down("Shift");
  await page.keyboard.down("Digit4");
  await page.keyboard.down("Digit4");
  await page.keyboard.up("Digit4");
  await page.keyboard.up("Shift");
  await expect.poll(() => ratingOf(page, id)).toBe(4);

  await page.keyboard.down("p");
  await page.keyboard.down("p");
  await page.keyboard.down("p");
  await page.keyboard.up("p");
  await expect.poll(() => prepColourOf(page, id)).toBe(2);
  await page.waitForTimeout(300);
  expect(await prepWrites(page)).toEqual([{ id, colour: 2 }]);
  const ratings = await page.evaluate(() => (window as unknown as { __ratingWrites: unknown[] }).__ratingWrites.length);
  expect(ratings - ratingsBefore).toBe(1);
});

test("a mark still being written does not move on once the DJ has chosen another track", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  const thirdId = (await options(page).nth(2).getAttribute("data-track"))!;
  const third = await options(page).nth(2).locator("span").first().innerText();
  await hold(page, "__holdPrepWrites");

  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
  await options(page).nth(2).click();
  await expect(loadedTitle(page)).toHaveText(third);
  await hold(page, "__releasePrepWrites");

  await expect.poll(() => prepColourOf(page, id)).toBe(5);
  await page.waitForTimeout(500);
  await expect(loadedTitle(page)).toHaveText(third);
  await expect(rowOf(page, thirdId)).toHaveAttribute("aria-selected", "true");
});

test("a Ctrl+Enter whose save finishes after the DJ chose another track saves but does not mark or move", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page);
  const third = await options(page).nth(2).locator("span").first().innerText();
  await hold(page, "__holdFieldWrites");

  const field = panel(page).getByRole("textbox", { name: "Title" });
  await field.fill("Saved Late");
  await field.press("Control+Enter");
  await options(page).nth(2).click();
  await expect(loadedTitle(page)).toHaveText(third);
  await hold(page, "__releaseFieldWrites");

  await expect(rowOf(page, id).locator("span").first()).toHaveText("Saved Late");
  await page.waitForTimeout(500);
  expect(await prepWrites(page)).toEqual([]);
  await expect(loadedTitle(page)).toHaveText(third);
});

test("a next page read that finishes after the DJ chose another track does not move or load", async ({ page }) => {
  await openIntake(page);
  const { id } = await choose(page, 199);
  const other = options(page).nth(197);
  const otherTitle = await other.locator("span").first().innerText();
  await hold(page, "__holdIntakeReads");

  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
  await expect.poll(() => prepColourOf(page, id)).toBe(5);
  await other.click();
  await expect(loadedTitle(page)).toHaveText(otherTitle);
  await hold(page, "__releaseIntakeReads");

  await page.waitForTimeout(500);
  await expect(loadedTitle(page)).toHaveText(otherTitle);
  await expect(page.getByTestId("intake-page")).toHaveText(/^1–200 of /);
});
