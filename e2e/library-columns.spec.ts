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

test("Prep colour is a stripe at the row's left edge in rekordbox's label colours", async ({ page }) => {
  await open(page);
  // The mock labels its tracks with every colour in turn, and some with none.
  const stripe = (name: string | RegExp) => page.getByRole("img", { name });
  const pinkRow = rows(page).filter({ has: stripe("Prep colour: Pink") }).first();
  const pink = pinkRow.getByRole("img", { name: "Prep colour: Pink" });
  await expect(pink).toBeVisible();
  expect(await background(pink)).toBe(await token(page, "--c-label-pink"));
  const rowBox = await pinkRow.boundingBox();
  const stripeBox = await pink.boundingBox();
  expect(stripeBox?.x).toBe(rowBox?.x);
  expect(stripeBox?.width).toBe(3);
  expect(stripeBox?.height).toBe(rowBox?.height);
  const green = rows(page).filter({ has: stripe("Prep colour: Green") }).first().getByRole("img", { name: "Prep colour: Green" });
  expect(await background(green)).toBe(await token(page, "--c-label-green"));
  await expect(rows(page).filter({ hasNot: stripe(/^Prep colour/) }).first()).toBeVisible();
  // Not a column of its own.
  expect(await headingNames(page)).not.toContain("color");
});

test("Energy is five pips coloured by level, and an unrated track shows unset pips", async ({ page }) => {
  await open(page);
  const rated = working(page).getByRole("img", { name: "Energy 3 of 5" }).first();
  await expect(rated).toBeVisible();
  const pips = rated.locator("[data-pip]");
  await expect(pips).toHaveCount(5);
  const green = await token(page, "--c-energy-3");
  for (const i of [0, 1, 2]) expect(await background(pips.nth(i))).toBe(green);
  const unset = await background(pips.nth(3));
  expect(unset).not.toBe(green);
  expect(await background(pips.nth(4))).toBe(unset);

  const hot = working(page).getByRole("img", { name: "Energy 5 of 5" }).first();
  expect(await background(hot.locator("[data-pip]").first())).toBe(await token(page, "--c-energy-5"));

  const unrated = working(page).getByRole("img", { name: "Energy not set" }).first();
  await expect(unrated).toBeVisible();
  for (let i = 0; i < 5; i++) expect(await background(unrated.locator("[data-pip]").nth(i))).toBe(unset);
});

test("clicking a pip sets Energy, and clicking the top lit pip clears it", async ({ page }) => {
  await open(page);
  const row = rows(page).filter({ has: page.getByRole("img", { name: "Energy not set" }) }).first();
  const title = await row.locator('[data-col="title"]').innerText();
  const same = rows(page).filter({ has: page.locator('[data-col="title"]', { hasText: title }) }).first();
  await same.locator("[data-pip='4']").click();
  await expect(same.getByRole("img", { name: "Energy 4 of 5" })).toBeVisible();
  await same.locator("[data-pip='4']").click();
  await expect(same.getByRole("img", { name: "Energy not set" })).toBeVisible();
});

test("MyTags show as tags", async ({ page }) => {
  await open(page);
  // The mock tags every third track Peak Time.
  await expect(working(page).locator('[data-col="myTag"]').getByText("Peak Time", { exact: true }).first()).toBeVisible();
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
