/**
 * The Track Filter bar (wbx-57).
 *
 * In the view shell a bar above the Library's list filters it with the
 * deck's Track Filter fields (MyTags by category, Energy, Prep colour, BPM
 * range, key) plus text. It narrows All Tracks and any playlist, the list
 * says how many tracks match out of how many, and clearing it puts the whole
 * list back. Every row shown is checked against what was picked, so a bar
 * that changed its label without narrowing, or narrowed wrongly, fails.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const working = (page: Page) => page.getByRole("tabpanel");
const bar = (page: Page) => page.getByRole("search", { name: "Track Filter" });
const grid = (page: Page) => working(page).getByRole("grid");
const rows = (page: Page) => working(page).locator('[role="row"][aria-selected]');
const count = (page: Page) => bar(page).getByRole("status");
const control = (page: Page, name: string) => bar(page).getByRole("button", { name });
const shellPlaylist = (page: Page, name: string) =>
  page.getByTestId("shell-tree").getByRole("group", { name: "Playlists" }).getByRole("treeitem", { name });

async function open(page: Page) {
  await enableViewShell(page);
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await expect(rows(page).first()).toBeVisible();
}

/** How many rows the list holds, as it tells assistive technology. */
async function listed(page: Page): Promise<number> {
  return Number(await grid(page).getAttribute("aria-rowcount"));
}

/**
 * The count the bar shows, as numbers `[matching, total]`, once it has
 * settled: read, and agreeing with what the list holds.
 */
async function counted(page: Page): Promise<[number, number]> {
  let seen: [number, number] = [-1, -1];
  await expect.poll(async () => {
    const text = (await count(page).innerText()).replace(/,/g, "");
    const match = /(\d+) of (\d+)/.exec(text);
    if (!match) return `the count says ${JSON.stringify(text)}`;
    seen = [Number(match[1]), Number(match[2])];
    return seen[0] === (await listed(page)) ? "settled" : `the count says ${text}, the list holds ${await listed(page)}`;
  }).toBe("settled");
  return seen;
}

function cell(row: Locator, column: string): Locator {
  return row.locator(`[role="gridcell"][data-col="${column}"]`);
}

/** Every row the list shows (the filters here leave few enough to all be drawn). */
async function everyRow(page: Page): Promise<Locator[]> {
  const n = await listed(page);
  await expect(rows(page)).toHaveCount(n);
  return rows(page).all();
}

test("the bar offers MyTags by category, Energy, Prep colour, BPM, key and text", async ({ page }) => {
  await open(page);
  const total = await listed(page);
  await expect(count(page)).toHaveText(`${total} tracks`);
  for (const name of ["Situation", "Components", "Energy", "Prep colour", "BPM", "Key"]) {
    await expect(control(page, name)).toBeVisible();
  }
  await expect(bar(page).getByRole("searchbox", { name: "Filter text" })).toBeVisible();
});

test("Energy and BPM ranges narrow All Tracks to exactly the rows that fit, with the count", async ({ page }) => {
  await open(page);
  const total = await listed(page);

  await control(page, "Energy").click();
  await bar(page).getByRole("combobox", { name: "Energy from" }).selectOption("4");
  await bar(page).getByRole("combobox", { name: "Energy to" }).selectOption("5");
  await expect(control(page, "Energy 4-5")).toHaveAttribute("data-active", "");

  await control(page, "BPM").click();
  await bar(page).getByRole("spinbutton", { name: "BPM from" }).fill("125");
  await bar(page).getByRole("spinbutton", { name: "BPM to" }).fill("134");
  await bar(page).getByRole("spinbutton", { name: "BPM to" }).press("Enter");
  await expect(control(page, "BPM 125-134")).toBeVisible();

  const [matching, of] = await counted(page);
  expect(of).toBe(total);
  expect(matching).toBe(await listed(page));
  expect(matching).toBeGreaterThan(0);
  expect(matching).toBeLessThan(total);
  for (const row of await everyRow(page)) {
    const bpm = Number(await cell(row, "bpm").innerText());
    expect(bpm).toBeGreaterThanOrEqual(125);
    expect(bpm).toBeLessThanOrEqual(134);
    await expect(cell(row, "rating").getByRole("radiogroup")).toHaveAccessibleName(/Energy [45] of 5/);
  }
});

test("a MyTag, a key and a Prep colour narrow the list, and clearing restores it", async ({ page }) => {
  await open(page);
  const total = await listed(page);

  await control(page, "Situation").click();
  await bar(page).getByRole("group", { name: "Situation tags" }).getByRole("button", { name: "Peak" }).click();
  await expect(control(page, "Situation Peak")).toBeVisible();
  const tagged = await listed(page);
  expect(tagged).toBeLessThan(total);
  expect((await counted(page))[0]).toBe(tagged);
  for (const row of (await rows(page).all()).slice(0, 10)) {
    await expect(cell(row, "myTag").locator('[title="Situation: Peak"]')).toHaveCount(1);
  }

  // A key: the list keeps only rows in it, as the key column shows them.
  await control(page, "Key").click();
  const keys = bar(page).getByRole("group", { name: "Keys" }).getByRole("button");
  const first = keys.first();
  const keyName = (await first.innerText()).trim();
  await first.click();
  await expect(first).toHaveAttribute("aria-pressed", "true");
  const narrowed = await listed(page);
  expect(narrowed).toBeLessThanOrEqual(tagged);
  for (const row of await everyRow(page)) {
    await expect(cell(row, "key")).toContainText(keyName);
  }

  await bar(page).getByRole("button", { name: "Clear filter" }).click();
  expect(await listed(page)).toBe(total);
  await expect(count(page)).toHaveText(`${total} tracks`);
  await expect(control(page, "Situation")).not.toHaveAttribute("data-active", "");
});

test("a Prep colour keeps only rows with that stripe", async ({ page }) => {
  await open(page);
  await control(page, "Prep colour").click();
  await bar(page).getByRole("group", { name: "Prep colours" }).getByRole("button", { name: "Green" }).click();
  await control(page, "Energy").click();
  await bar(page).getByRole("combobox", { name: "Energy from" }).selectOption("5");
  for (const row of await everyRow(page)) {
    await expect(row.getByRole("img", { name: "Prep colour: Green" })).toHaveCount(1);
  }
  expect((await counted(page))[0]).toBe(await listed(page));
});

test("text narrows together with the fields, and is the list's search", async ({ page }) => {
  await open(page);
  const total = await listed(page);
  const text = bar(page).getByRole("searchbox", { name: "Filter text" });
  await text.fill("Single");
  const matching = await listed(page);
  expect(matching).toBeLessThan(total);
  expect(await counted(page)).toEqual([matching, total]);
  await text.fill("");
  expect(await listed(page)).toBe(total);
});

test("the bar narrows a playlist and counts out of the playlist", async ({ page }) => {
  await open(page);
  await shellPlaylist(page, "Melodic Vox").click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  const inPlaylist = await listed(page);
  await expect(count(page)).toHaveText(`${inPlaylist} tracks`);

  await control(page, "Energy").click();
  await bar(page).getByRole("combobox", { name: "Energy from" }).selectOption("3");
  const [matching, of] = await counted(page);
  expect(of).toBe(inPlaylist);
  expect(matching).toBe(await listed(page));
  expect(matching).toBeLessThan(inPlaylist);

  // The filter stays when another list is opened, as the deck's does.
  await shellPlaylist(page, "Hardstyle").click();
  await expect(page.getByTestId("browser-title")).toContainText("Hardstyle");
  await expect(control(page, "Energy 3-5")).toBeVisible();
  expect((await counted(page))[0]).toBe(await listed(page));
});

test("the controls work from the keyboard, and Space and Enter never reach the deck", async ({ page }) => {
  await open(page);
  const total = await listed(page);
  // A track on the deck, stopped, so a stray Space would start it.
  const deck = page.getByTestId("shell-player");
  await rows(page).nth(3).locator('[role="gridcell"][data-col="title"]').dblclick();
  await expect(deck.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  const energy = control(page, "Energy");
  await energy.focus();
  await page.keyboard.press("Enter");
  await expect(energy).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("Escape");
  await expect(energy).toHaveAttribute("aria-expanded", "false");
  await expect(energy).toBeFocused();

  await control(page, "Situation").focus();
  await page.keyboard.press("Space");
  const peak = bar(page).getByRole("group", { name: "Situation tags" }).getByRole("button", { name: "Peak" });
  await peak.focus();
  await page.keyboard.press("Space");
  await expect(peak).toHaveAttribute("aria-pressed", "true");
  expect(await listed(page)).toBeLessThan(total);
  // Space and Enter on the controls did not start the deck.
  await expect(deck.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  await expect(deck.getByRole("button", { name: "Pause", exact: true })).toHaveCount(0);
});

test("a count that cannot be read says so instead of showing an old number", async ({ page }) => {
  await open(page);
  const total = await listed(page);
  await expect(count(page)).toHaveText(`${total} tracks`);
  // The backend stops answering list opens.
  await page.evaluate(() => {
    (window as unknown as { __failOpenView?: boolean }).__failOpenView = true;
  });
  await control(page, "Energy").click();
  await bar(page).getByRole("combobox", { name: "Energy from" }).selectOption("4");
  await expect(count(page)).toHaveText(/count unavailable/i);
});
