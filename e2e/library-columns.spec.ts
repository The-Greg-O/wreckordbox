/**
 * The Library view's columns and row look (ticket #55).
 *
 * In the view shell the Library lists a playlist or All Tracks with the
 * columns the DJ chooses tracks by: title, artist, BPM, key, Energy, MyTags,
 * time and date added, after the row's place in the list. Prep colour is a
 * stripe at the row's left edge, Energy is five pips coloured by level, and
 * artwork, album, label, bitrate and the preview waveform are optional
 * columns, off until the DJ turns them on from the headings' menu.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const working = (page: Page) => page.getByRole("tabpanel");
const headings = (page: Page) => working(page).getByRole("columnheader");
const rows = (page: Page) => working(page).locator('[role="row"][aria-selected]');
const columnMenu = (page: Page) => page.getByRole("menu", { name: "Columns" });

/** The headings as the DJ reads them, left to right. */
async function headingNames(page: Page): Promise<string[]> {
  return (await headings(page).allInnerTexts()).map((text) => text.replace(/[▲▼↑↓]/g, "").trim().toLowerCase());
}

async function open(page: Page) {
  await enableViewShell(page);
  await page.goto("/?writable=1");
  // The machine these run on is shared; opening can take a while.
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await expect(rows(page).first()).toBeVisible();
}

const DEFAULTS = ["#", "title", "artist", "bpm", "key", "energy", "mytags", "time", "date added"];

/** A colour as the browser computes it, for comparing with a token. */
async function background(locator: Locator): Promise<string> {
  return locator.evaluate((el) => getComputedStyle(el).backgroundColor);
}

async function token(page: Page, name: string): Promise<string> {
  return page.evaluate((property) => {
    const probe = document.createElement("div");
    probe.style.backgroundColor = `var(${property})`;
    document.body.append(probe);
    const colour = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return colour;
  }, name);
}

test("a fresh profile shows the default columns in the agreed order", async ({ page }) => {
  await open(page);
  expect(await headingNames(page)).toEqual(DEFAULTS);
});

test("rows and headings use the design's density", async ({ page }) => {
  await open(page);
  expect((await rows(page).first().boundingBox())?.height).toBe(24);
  expect((await headings(page).first().boundingBox())?.height).toBe(22);
});

test("Prep colour is a stripe at the row's left edge in rekordbox's own track colours", async ({ page }) => {
  await open(page);
  // The mock labels its tracks with every colour in turn, and some with none.
  const stripe = (name: string | RegExp) => page.getByRole("img", { name });
  const pinkRow = rows(page).filter({ has: stripe("Prep colour: Pink") }).first();
  const pink = pinkRow.getByRole("img", { name: "Prep colour: Pink" });
  await expect(pink).toBeVisible();
  // rekordbox's own track colours (cPink, cGreen in its ColorTheme), the
  // values the old list's Color column is pinned to.
  expect(await background(pink)).toBe("rgb(248, 112, 248)");
  const rowBox = await pinkRow.boundingBox();
  const stripeBox = await pink.boundingBox();
  expect(stripeBox?.x).toBe(rowBox?.x);
  expect(stripeBox?.width).toBe(3);
  expect(stripeBox?.height).toBe(rowBox?.height);
  const green = rows(page).filter({ has: stripe("Prep colour: Green") }).first().getByRole("img", { name: "Prep colour: Green" });
  expect(await background(green)).toBe("rgb(0, 224, 0)");
  await expect(rows(page).filter({ hasNot: stripe(/^Prep colour/) }).first()).toBeVisible();
  // Not a column of its own.
  expect(await headingNames(page)).not.toContain("color");
});

/** A row's Energy control: five pips the DJ can set from the mouse or the keyboard. */
const energy = (scope: Page | Locator, name: string | RegExp) => scope.getByRole("radiogroup", { name });

/** The mock's record of rating writes, and a track's rating as the mock holds it. */
async function ratingWrites(page: Page): Promise<{ ids: string[]; stars: number; refused: boolean }[]> {
  return page.evaluate(() => (window as unknown as { __ratingWrites: { ids: string[]; stars: number; refused: boolean }[] }).__ratingWrites);
}
async function storedRating(page: Page, id: string): Promise<number> {
  return page.evaluate((track) => (window as unknown as { __ratingOf: (id: string) => number }).__ratingOf(track), id);
}

/** The first row on show whose Energy is not set, held by its title so it is found again after a re-render. */
async function unratedRow(page: Page): Promise<Locator> {
  const row = rows(page).filter({ has: energy(page, "Energy not set") }).first();
  const title = await row.locator('[data-col="title"]').innerText();
  return rows(page).filter({ has: page.locator('[data-col="title"]', { hasText: title }) }).first();
}

test("Energy is five pips coloured by level, and an unrated track shows unset pips", async ({ page }) => {
  await open(page);
  const rated = energy(working(page), "Energy 3 of 5").first();
  await expect(rated).toBeVisible();
  const pips = rated.getByRole("radio");
  await expect(pips).toHaveCount(5);
  await expect(rated.getByRole("radio", { name: "Energy 3" })).toHaveAttribute("aria-checked", "true");
  await expect(rated.getByRole("radio", { name: "Energy 4" })).toHaveAttribute("aria-checked", "false");
  const green = await token(page, "--c-energy-3");
  for (const i of [0, 1, 2]) expect(await background(pips.nth(i))).toBe(green);
  const unset = await background(pips.nth(3));
  expect(unset).toBe(await token(page, "--c-energy-0"));
  expect(await background(pips.nth(4))).toBe(unset);

  const hot = energy(working(page), "Energy 5 of 5").first();
  expect(await background(hot.getByRole("radio").first())).toBe(await token(page, "--c-energy-5"));

  const unrated = energy(working(page), "Energy not set").first();
  await expect(unrated).toBeVisible();
  for (let i = 0; i < 5; i++) expect(await background(unrated.getByRole("radio").nth(i))).toBe(unset);
  await expect(unrated.getByRole("radio", { checked: true })).toHaveCount(0);
});

test("clicking a pip saves Energy to the Library, clicking it again clears it, and Undo puts each back", async ({ page }) => {
  await open(page);
  const row = await unratedRow(page);
  await row.getByRole("radio", { name: "Energy 4" }).click();
  await expect(energy(row, "Energy 4 of 5")).toBeVisible();
  await expect(page.getByTestId("shell-status")).toContainText("Rated 4 of 5.");
  const [write] = await ratingWrites(page);
  expect(write).toEqual({ ids: [expect.any(String)], stars: 4, refused: false });
  const id = write!.ids[0]!;
  expect(await storedRating(page, id)).toBe(4);

  await row.getByRole("radio", { name: "Energy 4" }).click();
  await expect(energy(row, "Energy not set")).toBeVisible();
  expect(await storedRating(page, id)).toBe(0);

  await page.evaluate(() => (window as unknown as { __menu: (id: string) => void }).__menu("undo"));
  await expect(energy(row, "Energy 4 of 5")).toBeVisible();
  expect(await storedRating(page, id)).toBe(4);
  await page.evaluate(() => (window as unknown as { __menu: (id: string) => void }).__menu("undo"));
  await expect(energy(row, "Energy not set")).toBeVisible();
  expect(await storedRating(page, id)).toBe(0);
});

test("Energy can be set and cleared from the keyboard", async ({ page }) => {
  await open(page);
  const row = await unratedRow(page);
  const three = row.getByRole("radio", { name: "Energy 3" });
  await three.focus();
  await expect(three).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(energy(row, "Energy 3 of 5")).toBeVisible();
  const id = (await ratingWrites(page))[0]!.ids[0]!;
  expect(await storedRating(page, id)).toBe(3);

  // The checked pip again clears it; Space works as Enter does.
  await three.focus();
  await page.keyboard.press(" ");
  await expect(energy(row, "Energy not set")).toBeVisible();
  expect(await storedRating(page, id)).toBe(0);
  // Neither key reached the deck: nothing started playing.
  await expect(page.getByRole("button", { name: "Pause" })).toHaveCount(0);
});

test("a pip sets Energy on its own row only, whatever else is selected", async ({ page }) => {
  await open(page);
  await rows(page).nth(0).locator('[data-col="title"]').click();
  await rows(page).nth(2).locator('[data-col="title"]').click({ modifiers: ["ControlOrMeta"] });
  await expect(rows(page).nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(rows(page).nth(2)).toHaveAttribute("aria-selected", "true");
  const target = rows(page).nth(1);
  const level = (await energy(target, /^Energy/).getAttribute("aria-label")) === "Energy 2 of 5" ? 4 : 2;
  await target.getByRole("radio", { name: `Energy ${level}` }).click();
  await expect(energy(target, `Energy ${level} of 5`)).toBeVisible();
  const writes = await ratingWrites(page);
  expect(writes).toHaveLength(1);
  expect(writes[0]!.ids).toHaveLength(1);
  expect(await storedRating(page, writes[0]!.ids[0]!)).toBe(level);
});

test("with Library Protection on a pip explains the lock and changes nothing", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
    advanced: { protectLibrary: true }, view: { viewShell: true },
  })));
  await page.goto("/?writable=1");
  await expect(rows(page).first()).toBeVisible({ timeout: 15_000 });
  const row = await unratedRow(page);
  await row.getByRole("radio", { name: "Energy 4" }).click();
  await expect(page.getByTestId("shell-status")).toContainText("Editing is locked by Library Protection");
  await expect(energy(row, "Energy not set")).toBeVisible();
  expect(await ratingWrites(page)).toEqual([]);
});

test("a write the Library refuses leaves Energy as it was", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/?writable=1&refuseRatingWrite=1");
  await expect(rows(page).first()).toBeVisible({ timeout: 15_000 });
  const row = await unratedRow(page);
  await row.getByRole("radio", { name: "Energy 4" }).click();
  await expect(page.getByTestId("shell-status")).toContainText("That could not be saved.");
  const [write] = await ratingWrites(page);
  expect(write?.refused).toBe(true);
  expect(await storedRating(page, write!.ids[0]!)).toBe(0);
  await expect(energy(row, "Energy not set")).toBeVisible();
});

test("MyTags show as tags", async ({ page }) => {
  await open(page);
  // The mock's My Tag record holds Peak (Situation) on every fourth track.
  const peak = working(page).locator('[data-col="myTag"] [data-tag="t-peak"]').first();
  await expect(peak).toHaveText("Peak");
  await expect(peak).toHaveAttribute("title", "Situation: Peak");
});

test("the optional columns are offered off, and turning one on shows it in its place and keeps it", async ({ page }) => {
  await open(page);
  await headings(page).filter({ hasText: /^artist/i }).click({ button: "right" });
  const menu = columnMenu(page);
  await expect(menu).toBeVisible();
  for (const name of ["Artwork", "Album", "Label", "Bitrate", "Preview"]) {
    await expect(menu.getByRole("menuitemcheckbox", { name })).toHaveAttribute("aria-checked", "false");
  }
  // Only what the Library view offers: no upstream catalogue.
  await expect(menu.getByRole("menuitemcheckbox", { name: "Composer" })).toHaveCount(0);

  await menu.getByRole("menuitemcheckbox", { name: "Album" }).click();
  await page.keyboard.press("Escape");
  const withAlbum = ["#", "title", "artist", "album", "bpm", "key", "energy", "mytags", "time", "date added"];
  expect(await headingNames(page)).toEqual(withAlbum);
  await expect(working(page).locator('[role="gridcell"][data-col="album"]').filter({ hasText: /\S/ }).first()).toBeVisible();

  await page.reload();
  await expect(rows(page).first()).toBeVisible();
  expect(await headingNames(page)).toEqual(withAlbum);
});

test("the old layout's columns are not changed by the Library view's", async ({ page }) => {
  // The switch is seeded once for this tab, so it can be turned off below.
  await page.addInitScript(() => {
    if (sessionStorage.getItem("seeded") !== null) return;
    sessionStorage.setItem("seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify({ advanced: { protectLibrary: false }, view: { viewShell: true } }));
  });
  await page.goto("/?writable=1");
  await expect(rows(page).first()).toBeVisible();
  await headings(page).filter({ hasText: /^artist/i }).click({ button: "right" });
  await columnMenu(page).getByRole("menuitemcheckbox", { name: "Bitrate" }).click();
  await page.keyboard.press("Escape");
  expect(await headingNames(page)).toContain("bitrate");

  // Switch the shell off: upstream's table keeps its own columns.
  await page.evaluate(() => {
    const p = JSON.parse(localStorage.getItem("rbl.preferences") ?? "{}");
    p.view = { ...p.view, viewShell: false };
    localStorage.setItem("rbl.preferences", JSON.stringify(p));
  });
  await page.reload();
  await expect(page.getByTestId("shell")).toHaveCount(0);
  await expect(page.getByRole("columnheader", { name: "Track Title" })).toBeVisible();
  await expect(page.getByRole("columnheader", { name: "Bitrate" })).toHaveCount(0);
});
