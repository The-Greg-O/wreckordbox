/**
 * The Intake tag panel (wbx-65): every dimension of the DJ's tags as
 * click-to-toggle tags, drawn from the description the Library service
 * reports, deck-facing ones marked and first. Every click is one step of
 * the shared Undo (a MyTag edit or the DJ's Fact), refused under Library
 * Protection, checked against the panel's version; "Mark Clean and next",
 * P and Skip wait for the track's tag writes.
 *
 * Mock backend (`src/ipc/mock-tag-panel.ts`): no flag = the mock's MyTag
 * categories (Situation: Peak, Warm-up; Components: Synth, Vocal);
 * `?tagSchema=five` five dimensions under unseen names; `?tagSchema=deckTree`
 * the genre tree on the deck (`&tagParents=0` without parents' MyTags),
 * vocals (one tag) and feel. Hooks: `__myTagsOf(id)` (MyTag names),
 * `__tagFactsOf(id)`, `__tagWrites()`, `__holdTagWrites()`,
 * `__releaseTagWrites()`, `__failNextTagWrite(message)`,
 * `__failTagPanelReads(on)`, `__moveToDeck(dimension)`,
 * `__outsideFact(id, dimension, value)`, `__prepWrites()`, `__ratingOf(id)`.
 */
import { expect, test, type Page } from "@playwright/test";
import { player, seedShell } from "./compactPlayer";

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const options = (page: Page) => page.getByRole("listbox", { name: "Intake queue" }).getByRole("option");
const tags = (page: Page) => page.getByRole("region", { name: "Tags" });
const dimension = (page: Page, label: string) => tags(page).getByRole("group", { name: label, exact: true });
const tag = (page: Page, label: string, name: string) => dimension(page, label).getByRole("button", { name, exact: true });
const loadedTitle = (page: Page) => player(page).getByTestId("compact-player-title");
const playing = (page: Page) => player(page).getByRole("button", { name: "Pause", exact: true });
const panel = (page: Page) => page.getByRole("region", { name: "Track" });

const call = <T>(page: Page, hook: string, ...args: unknown[]) =>
  page.evaluate(([name, rest]) => (window as unknown as Record<string, (...a: unknown[]) => unknown>)[name]!(...rest), [hook, args] as const) as Promise<T>;
const myTagsOf = (page: Page, id: string) => call<string[]>(page, "__myTagsOf", id);
const factsOf = (page: Page, id: string) => call<Record<string, string[] | string>>(page, "__tagFactsOf", id);

async function openIntake(page: Page, query = "", protect = false): Promise<void> {
  if (protect) {
    await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
      advanced: { protectLibrary: true }, view: { viewShell: true },
    })));
  } else {
    await seedShell(page);
  }
  await page.goto(`/${query}`);
  await expect(page.getByTestId("shell")).toBeVisible();
  await tab(page, "Intake").click();
  await expect(options(page).first()).toBeVisible();
}

async function choose(page: Page, at = 0): Promise<string> {
  const row = options(page).nth(at);
  const id = (await row.getAttribute("data-track"))!;
  const title = await row.locator("span").first().innerText();
  await row.click();
  await expect(loadedTitle(page)).toHaveText(title);
  await expect(tags(page)).toHaveAttribute("data-track", id);
  await expect(tags(page).getByRole("group").first()).toBeVisible();
  return id;
}

/** The dimensions as the DJ reads them, deck-facing ones starred. */
async function readOff(page: Page): Promise<string[]> {
  return tags(page).getByRole("group").evaluateAll((groups) => groups.map((group) =>
    `${group.getAttribute("data-deck") === "true" ? "*" : ""}${group.getAttribute("aria-label")}: ${
      [...group.querySelectorAll("button[data-tag]")].map((b) => b.textContent?.trim()).join(", ")}`));
}

/** The name of the first tag of `label` the shown track does not carry. */
async function offTag(page: Page, label: string): Promise<string> {
  return (await dimension(page, label).locator('button[data-tag][aria-pressed="false"]').first().innerText()).trim();
}

/** Ctrl+Z with nothing focused: the window's Undo. */
async function undo(page: Page): Promise<void> {
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press("Control+z");
}

test("with no Tag schema the panel is the MyTag categories, all deck-facing; a click sets and clears the MyTag", async ({ page }) => {
  await openIntake(page);
  const id = await choose(page);
  expect(await readOff(page)).toEqual(["*Situation: Peak, Warm-up", "*Components: Synth, Vocal"]);
  const name = await offTag(page, "Situation");

  await tag(page, "Situation", name).click();
  await expect.poll(() => myTagsOf(page, id)).toContain(name);
  await expect(tag(page, "Situation", name)).toHaveAttribute("aria-pressed", "true");

  await tag(page, "Situation", name).click();
  await expect.poll(() => myTagsOf(page, id)).not.toContain(name);
  await expect(tag(page, "Situation", name)).toHaveAttribute("aria-pressed", "false");
});

test("five dimensions with names the window has never seen: drawn as described, deck-facing first", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  await choose(page);
  expect(await readOff(page)).toEqual([
    "*Situation: Peak, Warm-up",
    "*Components: Synth, Vocal",
    "Genre: House, Deep, Acid, Techno",
    "Era: 90s, Modern",
    "Feel: Dark, Bright, Hypnotic",
  ]);
});

test("Undo takes back the DJ's last action whichever store it touched: a Fact tag, then the Energy", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);
  const before = await call<number>(page, "__ratingOf", id);
  const level = before === 3 ? 4 : 3;

  await panel(page).getByRole("radiogroup", { name: "Energy" }).getByRole("radio", { name: `Energy ${level}` }).click();
  await expect.poll(() => call<number>(page, "__ratingOf", id)).toBe(level);
  await tag(page, "Feel", "Dark").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ feel: ["dark"] });

  await undo(page);
  await expect.poll(() => factsOf(page, id)).toEqual({});
  await expect(tag(page, "Feel", "Dark")).toHaveAttribute("aria-pressed", "false");
  expect(await call<number>(page, "__ratingOf", id)).toBe(level);

  await undo(page);
  await expect.poll(() => call<number>(page, "__ratingOf", id)).toBe(before);
});

test("a deck tag: one Undo takes back one click", async ({ page }) => {
  await openIntake(page);
  const id = await choose(page);
  const name = await offTag(page, "Components");
  await tag(page, "Components", name).click();
  await expect.poll(() => myTagsOf(page, id)).toContain(name);
  await undo(page);
  await expect.poll(() => myTagsOf(page, id)).not.toContain(name);
  await expect(tag(page, "Components", name)).toHaveAttribute("aria-pressed", "false");
});

test("a deck-facing genre tree with tag_parents: parents ride along, an implied parent narrows, siblings keep it", async ({ page }) => {
  await openIntake(page, "?tagSchema=deckTree");
  const id = await choose(page);
  const genre = (names: string[]) => names.filter((n) => ["House", "> Deep", "> Acid", "Techno"].includes(n)).sort();

  await tag(page, "Sub-genre", "Deep").click();
  await expect.poll(async () => genre(await myTagsOf(page, id))).toEqual(["> Deep", "House"]);
  await expect(tag(page, "Sub-genre", "House")).toHaveAttribute("data-state", "implied");

  // The implied parent: narrows to the parent alone (as in a Fact tree).
  await tag(page, "Sub-genre", "House").click();
  await expect.poll(async () => genre(await myTagsOf(page, id))).toEqual(["House"]);
  await expect(tag(page, "Sub-genre", "House")).toHaveAttribute("data-state", "set");

  await tag(page, "Sub-genre", "Deep").click();
  await tag(page, "Sub-genre", "Acid").click();
  await expect.poll(async () => genre(await myTagsOf(page, id))).toEqual(["> Acid", "> Deep", "House"]);
  await tag(page, "Sub-genre", "Deep").click();
  await expect.poll(async () => genre(await myTagsOf(page, id))).toEqual(["> Acid", "House"]);
  await tag(page, "Sub-genre", "Acid").click();
  await expect.poll(async () => genre(await myTagsOf(page, id))).toEqual([]);

  // One click, one whole step of Undo: the child and its parent together.
  await tag(page, "Sub-genre", "Deep").click();
  await expect.poll(async () => genre(await myTagsOf(page, id))).toEqual(["> Deep", "House"]);
  await undo(page);
  await expect.poll(async () => genre(await myTagsOf(page, id))).toEqual([]);
});

test("a deck-facing genre tree without tag_parents writes only the child's MyTag", async ({ page }) => {
  await openIntake(page, "?tagSchema=deckTree&tagParents=0");
  const id = await choose(page);
  await tag(page, "Sub-genre", "Deep").click();
  await expect.poll(async () => (await myTagsOf(page, id)).filter((n) => n.includes("Deep") || n === "House")).toEqual(["> Deep"]);
  await expect(tag(page, "Sub-genre", "House")).toHaveAttribute("data-state", "implied");
});

test("a single-tag dimension replaces its tag and a second click clears it", async ({ page }) => {
  await openIntake(page, "?tagSchema=deckTree");
  const id = await choose(page);
  await tag(page, "Vocals", "Male").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ vocals: "male" });
  await tag(page, "Vocals", "Female").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ vocals: "female" });
  await expect(tag(page, "Vocals", "Male")).toHaveAttribute("aria-pressed", "false");
  await tag(page, "Vocals", "Female").click();
  await expect.poll(() => factsOf(page, id)).toEqual({});
});

test("in a Fact genre tree a child implies its parent, and turning the parent off narrows to it", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);
  await tag(page, "Genre", "Deep").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ genre: ["house-deep"] });
  await expect(tag(page, "Genre", "House")).toHaveAttribute("data-state", "implied");
  await tag(page, "Genre", "House").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ genre: ["house"] });
});

test("with Library Protection on, a click is refused, said, and nothing is written", async ({ page }) => {
  await openIntake(page, "?tagSchema=five", true);
  const id = await choose(page);
  const before = await myTagsOf(page, id);
  const name = await offTag(page, "Situation");

  await tag(page, "Situation", name).click();
  await tag(page, "Feel", "Dark").click();

  await expect(page.getByTestId("shell-status")).toContainText("Editing is locked by Library Protection");
  await expect(tags(page).getByTestId("intake-tags-failed")).toContainText("Library Protection");
  expect(await myTagsOf(page, id)).toEqual(before);
  expect(await factsOf(page, id)).toEqual({});
  await expect(tag(page, "Situation", name)).toHaveAttribute("aria-pressed", "false");
  await expect(tag(page, "Feel", "Dark")).toHaveAttribute("aria-pressed", "false");
});

test("tags toggle from the keyboard; a held key toggles once and Space never reaches the deck", async ({ page }) => {
  await openIntake(page);
  const id = await choose(page);
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();

  const peak = (await myTagsOf(page, id)).includes("Peak");
  await tag(page, "Situation", "Peak").focus();
  await page.keyboard.press("Space");
  await expect.poll(async () => (await myTagsOf(page, id)).includes("Peak")).toBe(!peak);
  await expect(playing(page)).toHaveCount(0);

  const warm = (await myTagsOf(page, id)).includes("Warm-up");
  await page.keyboard.press("Tab");
  await expect(tag(page, "Situation", "Warm-up")).toBeFocused();
  await page.keyboard.down("Enter");
  for (let i = 0; i < 5; i += 1) await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await expect.poll(async () => (await myTagsOf(page, id)).includes("Warm-up")).toBe(!warm);
  await page.waitForTimeout(300);
  expect((await myTagsOf(page, id)).includes("Warm-up")).toBe(!warm);
  expect((await call<unknown[]>(page, "__tagWrites")).length).toBe(2);
  await expect(playing(page)).toHaveCount(0);
});

test("fast clicks while writes are slow: shown at once, written one at a time, ending where the DJ left them", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);
  await call(page, "__holdTagWrites");
  for (const name of ["Dark", "Bright", "Hypnotic", "Bright"]) await tag(page, "Feel", name).click();
  await expect(tag(page, "Feel", "Hypnotic")).toHaveAttribute("aria-pressed", "true");
  await expect(tag(page, "Feel", "Bright")).toHaveAttribute("aria-pressed", "false");
  await call(page, "__releaseTagWrites");

  await expect.poll(() => factsOf(page, id)).toEqual({ feel: ["dark", "hypnotic"] });
  const writes = await call<{ terms: string[] }[]>(page, "__tagWrites");
  expect(writes.length).toBeLessThanOrEqual(2);
  expect(writes.at(-1)?.terms).toEqual(["dark", "hypnotic"]);
});

test("a failed write stays said after an unrelated one succeeds, until retried", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);
  await call(page, "__holdTagWrites");
  await call(page, "__failNextTagWrite", "The Sidecar is busy.");
  await tag(page, "Feel", "Dark").click();
  await tag(page, "Era", "90s").click();
  await call(page, "__releaseTagWrites");

  await expect.poll(() => factsOf(page, id)).toEqual({ era: ["nineties"] });
  const failed = tags(page).getByTestId("intake-tags-failed");
  await expect(failed).toContainText("Feel");
  await expect(failed).toContainText("The Sidecar is busy.");
  await expect(tag(page, "Feel", "Dark")).toHaveAttribute("aria-pressed", "false");
  await page.waitForTimeout(500);
  await expect(failed).toContainText("The Sidecar is busy.");

  await failed.getByRole("button", { name: "Retry" }).click();
  await expect.poll(() => factsOf(page, id)).toEqual({ era: ["nineties"], feel: ["dark"] });
  await expect(tags(page).getByTestId("intake-tags-failed")).toHaveCount(0);
});

test("Mark Clean and next waits for the track's tag writes, and a failed one stops it and stays said after moving on", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page, 0);
  await call(page, "__holdTagWrites");
  await tag(page, "Feel", "Dark").click();
  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
  await page.waitForTimeout(300);
  expect(await call<unknown[]>(page, "__prepWrites")).toEqual([]);
  await call(page, "__releaseTagWrites");
  // Written, then marked, then moved on.
  await expect.poll(() => call<{ id: string }[]>(page, "__prepWrites")).toEqual([{ id, colour: 5 }]);
  expect(await factsOf(page, id)).toEqual({ feel: ["dark"] });
  await expect(tags(page)).not.toHaveAttribute("data-track", id);

  // Now a failure: the mark is not made.
  const next = (await tags(page).getAttribute("data-track"))!;
  await call(page, "__holdTagWrites");
  await call(page, "__failNextTagWrite", "Refused for the test.");
  await tag(page, "Feel", "Bright").click();
  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
  await call(page, "__releaseTagWrites");
  await expect(panel(page).getByTestId("intake-edit-failed")).toContainText("Refused for the test.");
  expect((await call<{ id: string }[]>(page, "__prepWrites")).map((w) => w.id)).not.toContain(next);
  await expect(tags(page)).toHaveAttribute("data-track", next);

  // The DJ moves on by choosing another row: the failure is still listed, naming the track.
  await options(page).nth(2).click();
  await expect(page.getByTestId("intake-tag-failures")).toContainText("Refused for the test.");
  await expect(page.getByTestId("intake-tag-failures").locator(`[data-track="${next}"]`)).toHaveCount(1);
});

test("a click on a panel the schema changed under is refused, said, and the panel reads again", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);
  await expect(dimension(page, "Feel")).toHaveAttribute("data-deck", "false");

  await call(page, "__moveToDeck", "feel");
  await tag(page, "Feel", "Dark").click();

  await expect(tags(page).getByTestId("intake-tags-failed")).toContainText("changed");
  expect(await factsOf(page, id)).toEqual({});
  await expect(dimension(page, "Feel")).toHaveAttribute("data-deck", "true");
});

test("a Fact recorded by another client shows at once, and a value no tag stands for is kept until taken off", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);

  await call(page, "__outsideFact", id, "feel", ["dark", "retired-term"]);
  await expect(tag(page, "Feel", "Dark")).toHaveAttribute("aria-pressed", "true");
  const stray = dimension(page, "Feel").locator('[data-unmapped="retired-term"]');
  await expect(stray).toBeVisible();

  await tag(page, "Feel", "Bright").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ feel: ["dark", "bright", "retired-term"] });

  await stray.click();
  await expect.poll(() => factsOf(page, id)).toEqual({ feel: ["dark", "bright"] });
  await expect(stray).toHaveCount(0);
});

test("a click whose write lands after another track was chosen is that track's, not the new one's", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const first = await choose(page, 0);
  await call(page, "__holdTagWrites");
  await tag(page, "Era", "90s").click();
  const second = await choose(page, 1);
  await expect(tag(page, "Era", "90s")).toHaveAttribute("aria-pressed", "false");
  await call(page, "__releaseTagWrites");
  await expect.poll(() => factsOf(page, first)).toEqual({ era: ["nineties"] });
  expect(await factsOf(page, second)).toEqual({});
});

test("the palette's Tag Track… picks any dimension's tag by search, and focus goes back", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);
  const row = tag(page, "Era", "Modern");
  await row.focus();

  await page.keyboard.press("Control+Shift+P");
  await expect(page.locator("[data-command-palette]")).toBeVisible();
  await page.keyboard.type("Tag Track");
  await page.keyboard.press("Enter");
  const picker = page.getByRole("dialog", { name: "Tag the track" });
  await expect(picker).toBeVisible();
  await page.keyboard.type("feel hyp");
  await expect(picker.getByRole("option")).toHaveCount(1);
  await page.keyboard.press("Enter");

  await expect(picker).toHaveCount(0);
  await expect.poll(() => factsOf(page, id)).toEqual({ feel: ["hypnotic"] });
  await expect(row).toBeFocused();

  // T opens it straight from the view; Escape closes, nothing changes.
  await page.keyboard.press("t");
  await expect(picker).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(picker).toHaveCount(0);
  await expect(row).toBeFocused();
  expect(await factsOf(page, id)).toEqual({ feel: ["hypnotic"] });
});

test("a panel that cannot be read says so, with Retry", async ({ page }) => {
  await openIntake(page);
  await call(page, "__failTagPanelReads", true);
  await options(page).nth(0).click();
  await expect(tags(page).getByTestId("intake-tags-unavailable")).toContainText("did not answer");
  await call(page, "__failTagPanelReads", false);
  await tags(page).getByRole("button", { name: "Retry" }).click();
  await expect(dimension(page, "Situation")).toBeVisible();
});
