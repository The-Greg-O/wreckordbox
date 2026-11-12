/**
 * The Intake tag panel (wbx-65): every dimension of the DJ's tags as
 * click-to-toggle tags, drawn from the description the Library service
 * reports. Deck-facing dimensions are marked and listed first and toggle
 * through the window's MyTag edit (one undoable step each, refused under
 * Library Protection); the others are the DJ's Facts.
 *
 * Mock backend (`src/ipc/mock-tag-panel.ts`): without a flag, the mock's
 * two MyTag categories (Situation: Peak, Warm-up; Components: Synth,
 * Vocal); `?tagSchema=five` a schema of five dimensions under names the
 * window has never seen. `__myTagsOf(id)`, `__tagFactsOf(id)` read what
 * the mock holds; `__tagFactWrites()`, `__holdTagFactWrites()`,
 * `__releaseTagFactWrites()`, `__failTagPanelReads(on)`.
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

const call = <T>(page: Page, hook: string, ...args: unknown[]) =>
  page.evaluate(([name, rest]) => (window as unknown as Record<string, (...a: unknown[]) => unknown>)[name]!(...rest), [hook, args] as const) as Promise<T>;
const myTagsOf = (page: Page, id: string) => call<string[]>(page, "__myTagsOf", id);
const factsOf = (page: Page, id: string) => call<Record<string, string[]>>(page, "__tagFactsOf", id);

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
  return id;
}

/** The dimensions as the DJ reads them, deck-facing ones starred. */
async function readOff(page: Page): Promise<string[]> {
  return tags(page).getByRole("group").evaluateAll((groups) => groups.map((group) =>
    `${group.getAttribute("data-deck") === "true" ? "*" : ""}${group.getAttribute("aria-label")}: ${
      [...group.querySelectorAll("button[data-tag]")].map((b) => b.textContent?.trim()).join(", ")}`));
}

/** The name and MyTag id of the first tag of `label` the shown track does not carry. */
async function offTag(page: Page, label: string): Promise<{ name: string; myTag: string }> {
  const chip = dimension(page, label).locator('button[aria-pressed="false"]').first();
  return { name: (await chip.innerText()).trim(), myTag: (await chip.getAttribute("data-my-tag"))! };
}

/** Ctrl+Z with nothing focused: the window's Undo. */
async function undo(page: Page): Promise<void> {
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press("Control+z");
}

test("with no Tag schema the panel is the MyTag categories, all marked as reaching the deck; a click sets and clears", async ({ page }) => {
  await openIntake(page);
  const id = await choose(page);
  expect(await readOff(page)).toEqual(["*Situation: Peak, Warm-up", "*Components: Synth, Vocal"]);
  const { name, myTag } = await offTag(page, "Situation");
  expect(await myTagsOf(page, id)).not.toContain(myTag);

  await tag(page, "Situation", name).click();
  await expect.poll(() => myTagsOf(page, id)).toContain(myTag);
  await expect(tag(page, "Situation", name)).toHaveAttribute("aria-pressed", "true");

  await tag(page, "Situation", name).click();
  await expect.poll(() => myTagsOf(page, id)).not.toContain(myTag);
  await expect(tag(page, "Situation", name)).toHaveAttribute("aria-pressed", "false");
});

test("one Undo takes back one tag click, and the panel follows", async ({ page }) => {
  await openIntake(page);
  const id = await choose(page);

  const { name, myTag } = await offTag(page, "Components");
  await tag(page, "Components", name).click();
  await expect.poll(() => myTagsOf(page, id)).toContain(myTag);
  await expect(tag(page, "Components", name)).toHaveAttribute("aria-pressed", "true");

  await undo(page);
  await expect.poll(() => myTagsOf(page, id)).not.toContain(myTag);
  await expect(tag(page, "Components", name)).toHaveAttribute("aria-pressed", "false");
});

test("five dimensions with names the window has never seen: drawn as described, deck-facing first", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);

  expect(await readOff(page)).toEqual([
    "*Situation: Peak, Warm-up",
    "*Components: Synth, Vocal",
    "Genre: House, Deep, Acid, Techno",
    "Era: 90s, Modern",
    "Feel: Dark, Bright, Hypnotic",
  ]);

  // A deck-facing dimension still edits the MyTag.
  const { name, myTag } = await offTag(page, "Situation");
  await tag(page, "Situation", name).click();
  await expect.poll(() => myTagsOf(page, id)).toContain(myTag);

  // The others are the DJ's Facts.
  await tag(page, "Feel", "Dark").click();
  await tag(page, "Feel", "Hypnotic").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ feel: ["dark", "hypnotic"] });
  await expect(tag(page, "Feel", "Hypnotic")).toHaveAttribute("aria-pressed", "true");
});

test("in the genre tree a child implies its parent, and turning the parent off takes the child off", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);

  await tag(page, "Genre", "House").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ genre: ["house"] });
  // A child names the more specific genre: the parent is implied, not stored.
  await tag(page, "Genre", "Deep").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ genre: ["house-deep"] });
  await expect(tag(page, "Genre", "House")).toHaveAttribute("data-state", "implied");
  await expect(tag(page, "Genre", "Deep")).toHaveAttribute("data-state", "set");

  await tag(page, "Genre", "House").click();
  await expect.poll(() => factsOf(page, id)).toEqual({ genre: [] });
  await expect(tag(page, "Genre", "House")).toHaveAttribute("data-state", "off");
});

test("with Library Protection on, a deck tag click is refused, said, and nothing is written", async ({ page }) => {
  await openIntake(page, "", true);
  const id = await choose(page);
  const before = await myTagsOf(page, id);
  const { name } = await offTag(page, "Situation");

  await tag(page, "Situation", name).click();

  await expect(page.getByTestId("shell-status")).toContainText("Editing is locked by Library Protection");
  await expect(tags(page).getByTestId("intake-tags-failed")).toContainText("Library Protection");
  expect(await myTagsOf(page, id)).toEqual(before);
  await expect(tag(page, "Situation", name)).toHaveAttribute("aria-pressed", "false");
});

test("tags toggle from the keyboard; a held key toggles once and Space never reaches the deck", async ({ page }) => {
  await openIntake(page);
  const id = await choose(page);
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();

  const peak = (await myTagsOf(page, id)).includes("t-peak");
  await tag(page, "Situation", "Peak").focus();
  await page.keyboard.press("Space");
  await expect(tag(page, "Situation", "Peak")).toHaveAttribute("aria-pressed", String(!peak));
  await expect.poll(async () => (await myTagsOf(page, id)).includes("t-peak")).toBe(!peak);
  await expect(playing(page)).toHaveCount(0);

  // Tab to the next tag, then a held Enter: one toggle, not one per repeat.
  const warm = (await myTagsOf(page, id)).includes("t-warm");
  await page.keyboard.press("Tab");
  await expect(tag(page, "Situation", "Warm-up")).toBeFocused();
  await page.keyboard.down("Enter");
  for (let i = 0; i < 5; i += 1) await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await expect(tag(page, "Situation", "Warm-up")).toHaveAttribute("aria-pressed", String(!warm));
  await expect.poll(async () => (await myTagsOf(page, id)).includes("t-warm")).toBe(!warm);
  // Settled: still one toggle after any repeats would have landed.
  await page.waitForTimeout(300);
  expect((await myTagsOf(page, id)).includes("t-warm")).toBe(!warm);
  await expect(playing(page)).toHaveCount(0);
});

test("fast clicks while writes are slow: each shows at once, writes go one at a time and end where the DJ left them", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const id = await choose(page);
  await call(page, "__holdTagFactWrites");

  for (const name of ["Dark", "Bright", "Hypnotic", "Bright"]) await tag(page, "Feel", name).click();

  // Shown as the DJ asked, before anything is written.
  await expect(tag(page, "Feel", "Dark")).toHaveAttribute("aria-pressed", "true");
  await expect(tag(page, "Feel", "Bright")).toHaveAttribute("aria-pressed", "false");
  await expect(tag(page, "Feel", "Hypnotic")).toHaveAttribute("aria-pressed", "true");
  await call(page, "__releaseTagFactWrites");

  await expect.poll(() => factsOf(page, id)).toEqual({ feel: ["dark", "hypnotic"] });
  const writes = await call<{ terms: string[] }[]>(page, "__tagFactWrites");
  // The first write was on its way; the clicks behind it are one write.
  expect(writes.length).toBeLessThanOrEqual(2);
  expect(writes.at(-1)?.terms).toEqual(["dark", "hypnotic"]);
  await expect(tag(page, "Feel", "Hypnotic")).toHaveAttribute("aria-pressed", "true");
});

test("a click whose write lands after another track was chosen is that track's, not the new one's", async ({ page }) => {
  await openIntake(page, "?tagSchema=five");
  const first = await choose(page, 0);
  await call(page, "__holdTagFactWrites");
  await tag(page, "Era", "90s").click();

  const second = await choose(page, 1);
  await expect(tag(page, "Era", "90s")).toHaveAttribute("aria-pressed", "false");
  await call(page, "__releaseTagFactWrites");

  await expect.poll(() => factsOf(page, first)).toEqual({ era: ["nineties"] });
  expect(await factsOf(page, second)).toEqual({});
  await expect(tag(page, "Era", "90s")).toHaveAttribute("aria-pressed", "false");
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
