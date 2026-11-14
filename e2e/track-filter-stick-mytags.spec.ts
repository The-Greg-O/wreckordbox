/**
 * MyTags on a Stick playlist (wbx-45.12).
 *
 * On a Stick's list the Track Filter bar offers the Stick's own MyTag
 * categories and tags, and its picks narrow the list by the Stick's own
 * MyTags: All or Any within a category, every category together. Picks made
 * on the Library that the Stick does not hold are left out there, said so,
 * and kept for the Library.
 *
 * The mock Stick (`src/ipc/mock-stick-mytags.ts`): "Vibe" (Dark, Uplifting)
 * and "Situation" (Peak, Warm-up). "Now Playing Test" holds B2ME (Uplifting,
 * Peak), Spectrum (Dark, Uplifting, Peak), Lost Without You (Dark, Warm-up)
 * and Gravity (none); "Melodic Vox" the last three. The Library is the
 * filter fixture (`?filterFixture=1`): Situation Peak on even tracks,
 * Components Synth when i mod 4 is 0.
 */
import { expect, test, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const working = (page: Page) => page.getByRole("tabpanel");
const bar = (page: Page) => page.getByRole("search", { name: "Track Filter" });
const rows = (page: Page) => working(page).locator('[role="row"][aria-selected]');
const titles = (page: Page) => rows(page).locator('[data-col="title"]');
const count = (page: Page) => bar(page).getByRole("status");
const control = (page: Page, name: string) =>
  bar(page).getByRole("button", { name: new RegExp(`^${name}( (?!match|±).*)?$`) });
const panelGroup = (page: Page, name: string) => bar(page).getByRole("group", { name, exact: true });
const tree = (page: Page) => page.getByTestId("shell-tree");

const B2ME = "B2ME (feat. JordinLaine) (Original Mix)";
const SPECTRUM = "Spectrum (Extended Mix)";
const LOST = "Lost Without You";
const GRAVITY = "Gravity";

async function open(page: Page) {
  await enableViewShell(page);
  await page.goto("/?writable=1&filterFixture=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await tree(page).getByRole("treeitem", { name: /^All Tracks/ }).click();
  await expect(page.getByTestId("browser-title")).toContainText("All Tracks");
  await expect(rows(page).first()).toBeVisible();
}

async function stickList(page: Page, name: string) {
  await tree(page).getByRole("group", { name: "Stick TEST" }).getByRole("treeitem", { name }).click();
  await expect(page.getByTestId("browser-title")).toContainText(name);
}

async function pick(page: Page, category: string, tag: string) {
  if (await control(page, category).first().getAttribute("aria-expanded") !== "true") await control(page, category).first().click();
  await panelGroup(page, `${category} tags`).getByRole("button", { name: tag, exact: true }).click();
}

async function matchAny(page: Page, category: string) {
  if (await control(page, category).first().getAttribute("aria-expanded") !== "true") await control(page, category).first().click();
  await panelGroup(page, `${category} match`).getByRole("button", { name: "Any" }).click();
}

async function shows(page: Page, expected: string[], of: number) {
  await expect(count(page)).toHaveText(`${expected.length} of ${of} tracks`);
  await expect(titles(page)).toHaveText(expected);
}

test("on a Stick list the bar offers the Stick's own MyTag categories and tags", async ({ page }) => {
  await open(page);
  // The Library's categories first.
  await expect(control(page, "Components")).toBeVisible();
  await stickList(page, "Now Playing Test");
  await expect(count(page)).toHaveText("4 tracks");
  await expect(control(page, "Vibe")).toBeVisible();
  await expect(control(page, "Situation")).toBeVisible();
  await expect(control(page, "Components")).toHaveCount(0);
  await control(page, "Vibe").click();
  await expect(panelGroup(page, "Vibe tags").getByRole("button")).toHaveText(["Dark", "Uplifting"]);
  await control(page, "Situation").click();
  await expect(panelGroup(page, "Situation tags").getByRole("button")).toHaveText(["Peak", "Warm-up"]);
  await expect(bar(page)).not.toContainText("not read");
});

test("a Stick playlist is narrowed by its MyTags: All or Any within a category, every category together", async ({ page }) => {
  await open(page);
  await stickList(page, "Now Playing Test");
  await pick(page, "Vibe", "Dark");
  await shows(page, [SPECTRUM, LOST], 4);
  await pick(page, "Vibe", "Uplifting");
  await shows(page, [SPECTRUM], 4);
  await matchAny(page, "Vibe");
  await shows(page, [B2ME, SPECTRUM, LOST], 4);
  await pick(page, "Situation", "Peak");
  await shows(page, [B2ME, SPECTRUM], 4);
  await pick(page, "Situation", "Warm-up");
  await shows(page, [], 4);
  await matchAny(page, "Situation");
  await shows(page, [B2ME, SPECTRUM, LOST], 4);
  // Kept on another of the Stick's playlists, counted out of its own.
  await stickList(page, "Melodic Vox");
  await shows(page, [SPECTRUM, LOST], 3);
  await bar(page).getByRole("button", { name: "Clear filter" }).click();
  await expect(count(page)).toHaveText("3 tracks");
  await expect(titles(page)).toHaveText([SPECTRUM, LOST, GRAVITY]);
});

test("Library picks the Stick does not hold are left out there, said so, and kept for the Library", async ({ page }) => {
  await open(page);
  await control(page, "Situation").click();
  await panelGroup(page, "Situation tags").getByRole("button", { name: "Peak", exact: true }).click();
  await control(page, "Components").click();
  await panelGroup(page, "Components tags").getByRole("button", { name: "Synth", exact: true }).click();
  await expect(count(page)).toHaveText("8 of 30 tracks");

  await stickList(page, "Now Playing Test");
  // Situation: Peak is the Stick's own Peak; Components is not on the Stick.
  await shows(page, [B2ME, SPECTRUM], 4);
  await expect(bar(page).getByRole("note")).toHaveText("Not on this Stick, left out here: Components");
  await expect(control(page, "Situation Peak")).toHaveAttribute("data-active", "");

  await tree(page).getByRole("treeitem", { name: /^All Tracks/ }).click();
  await expect(page.getByTestId("browser-title")).toContainText("All Tracks");
  await expect(count(page)).toHaveText("8 of 30 tracks");
  await expect(bar(page).getByRole("note")).toHaveCount(0);
});

test("screenshots of a Stick list narrowed by its MyTags", async ({ page }) => {
  const dir = process.env.SHOTS_DIR;
  test.skip(!dir, "set SHOTS_DIR to write the screenshots");
  await page.setViewportSize({ width: 1280, height: 800 });
  await open(page);
  await stickList(page, "Now Playing Test");
  await pick(page, "Vibe", "Dark");
  await shows(page, [SPECTRUM, LOST], 4);
  await page.screenshot({ path: `${dir}/stick-mytags-1280.png` });
  // Half width: the tree gives way; the bar and its panel stay.
  await page.setViewportSize({ width: 640, height: 800 });
  await shows(page, [SPECTRUM, LOST], 4);
  await page.screenshot({ path: `${dir}/stick-mytags-640.png` });
});
