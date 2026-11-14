/**
 * The Track Filter bar (wbx-57).
 *
 * In the view shell a bar above the Library's list filters it with the
 * deck's Track Filter fields (MyTags by category, Energy, Prep colour, BPM,
 * key, Key match and BPM ±6% against the loaded track) plus text. It narrows
 * All Tracks, a playlist, an intelligent playlist and a Stick's playlist,
 * the list says how many tracks match out of how many, and clearing it puts
 * the whole list back.
 *
 * Every test runs on the mock's constructed Library (`?filterFixture=1`,
 * `src/ipc/mock-filter-fixture.ts`) and names exactly the tracks a filter
 * must leave, worked out from that fixture's definition: track i of 0 to 29
 * is "Bar ii", BPM 120.00 + 0.25 i, key 8A, 9A, 11B, 8A, Am by i mod 5 (28 is
 * Open Key 1m, 29 is "Mystery"), Energy (i / 2) mod 6, Prep colour i mod 9
 * (0 none, 5 Green), Situation Peak when i is even and Warm-up when i mod 5
 * is 0, Components Synth when i mod 4 is 0 and Vocal when i mod 7 is 0,
 * genre Deep House when i mod 3 is 1. "Melodic Vox" holds 3 to 20, the
 * intelligent playlist "Fresh 128s" the Deep House tracks, "Eurodance"
 * nothing.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const working = (page: Page) => page.getByRole("tabpanel");
const bar = (page: Page) => page.getByRole("search", { name: "Track Filter" });
const grid = (page: Page) => working(page).getByRole("grid");
const rows = (page: Page) => working(page).locator('[role="row"][aria-selected]');
const count = (page: Page) => bar(page).getByRole("status");
/**
 * A control by its name, picked or not ("Energy" is also "Energy 4-5"), but
 * never another control whose name starts the same ("Key" is not "Key
 * match", "BPM" is not "BPM ±6%").
 */
const control = (page: Page, name: string) => {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return bar(page).getByRole("button", { name: new RegExp(`^${escaped}( (?!match|±).*)?$`) });
};
const panelGroup = (page: Page, name: string) => bar(page).getByRole("group", { name, exact: true });
const tree = (page: Page) => page.getByTestId("shell-tree");
const shellPlaylist = (page: Page, name: string) =>
  tree(page).getByRole("group", { name: "Playlists" }).getByRole("treeitem", { name, exact: true });

const bars = (...ids: number[]) => ids.map((i) => `Bar ${String(i).padStart(2, "0")}`);

async function allTracks(page: Page) {
  await tree(page).getByRole("treeitem", { name: /^All Tracks/ }).click();
  await expect(page.getByTestId("browser-title")).toContainText("All Tracks");
  await expect(rows(page).first()).toBeVisible();
}

async function open(page: Page, extra = "") {
  await enableViewShell(page);
  await page.goto(`/?writable=1&filterFixture=1${extra}`);
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await allTracks(page);
}

/** How many rows the list holds, as it tells assistive technology. */
async function listed(page: Page): Promise<number> {
  return Number(await grid(page).getAttribute("aria-rowcount"));
}

/**
 * The titles the list shows, once its rows and the bar's count agree on
 * `expected`'s length: every row of a filter that leaves few enough to be
 * drawn at once.
 */
async function shows(page: Page, expected: string[], of: number) {
  await expect.poll(() => listed(page)).toBe(expected.length);
  await expect(count(page)).toHaveText(`${expected.length} of ${of} tracks`);
  await expect(rows(page)).toHaveCount(expected.length);
  const titles = await rows(page).locator('[role="gridcell"][data-col="title"]').allInnerTexts();
  expect(titles.map((t) => t.trim())).toEqual(expected);
}

async function energy(page: Page, from: string, to?: string) {
  if (await control(page, "Energy").getAttribute("aria-expanded") !== "true") await control(page, "Energy").click();
  await bar(page).getByRole("combobox", { name: "Energy from" }).selectOption(from);
  if (to !== undefined) await bar(page).getByRole("combobox", { name: "Energy to" }).selectOption(to);
}

async function bpm(page: Page, from: string, to: string) {
  if (await control(page, "BPM").getAttribute("aria-expanded") !== "true") await control(page, "BPM").click();
  await bar(page).getByRole("spinbutton", { name: "BPM from" }).fill(from);
  await bar(page).getByRole("spinbutton", { name: "BPM to" }).fill(to);
  await bar(page).getByRole("spinbutton", { name: "BPM to" }).press("Enter");
}

async function pick(page: Page, controlName: string, groupName: string, choice: string) {
  if (await control(page, controlName).first().getAttribute("aria-expanded") !== "true") await control(page, controlName).first().click();
  await panelGroup(page, groupName).getByRole("button", { name: choice, exact: true }).click();
}

test("the bar offers MyTags by category, Energy, Prep colour, BPM, key, the loaded-track matches and text", async ({ page }) => {
  await open(page);
  await expect(count(page)).toHaveText("30 tracks");
  for (const name of ["Situation", "Components", "Energy", "Prep colour", "BPM", "Key", "Key match", "BPM ±6%"]) {
    await expect(control(page, name)).toBeVisible();
  }
  await expect(bar(page).getByRole("textbox", { name: "Filter text" })).toBeVisible();
});

test("Energy: any is everything, 1 to 5 is every rated track, unrated is its own choice", async ({ page }) => {
  await open(page);
  await energy(page, "1", "5");
  await expect.poll(() => listed(page)).toBe(24);
  await expect(count(page)).toHaveText("24 of 30 tracks");
  // Any Energy, Unrated only.
  await energy(page, "");
  await expect.poll(() => listed(page)).toBe(30);
  await bar(page).getByRole("button", { name: "Unrated" }).click();
  await shows(page, bars(0, 1, 12, 13, 24, 25), 30);
  // Unrated with Energy 5 is both.
  await energy(page, "5", "5");
  await shows(page, bars(0, 1, 10, 11, 12, 13, 22, 23, 24, 25), 30);
  await expect(control(page, "Energy 5, unrated")).toBeVisible();
});

test("Prep colour None is the tracks with no colour, together with Green", async ({ page }) => {
  await open(page);
  await pick(page, "Prep colour", "Prep colours", "None");
  await pick(page, "Prep colour", "Prep colours", "Green");
  await shows(page, bars(0, 5, 9, 14, 18, 23, 27), 30);
  for (const row of await rows(page).all()) {
    const stripe = row.getByRole("img", { name: /Prep colour/ });
    const title = (await row.locator('[data-col="title"]').innerText()).trim();
    if (["Bar 05", "Bar 14", "Bar 23"].includes(title)) await expect(stripe).toHaveAccessibleName("Prep colour: Green");
    else await expect(stripe).toHaveCount(0);
  }
});

test("a key is every spelling of it in every list; an unknown key is exact", async ({ page }) => {
  await open(page);
  await shellPlaylist(page, "Melodic Vox").click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  // Melodic Vox holds 8A and Am, offered as one key.
  await control(page, "Key").click();
  const keys = panelGroup(page, "Keys").getByRole("button");
  await expect(keys).toHaveText(["8A", "9A", "11B"]);
  await keys.first().click();
  await shows(page, bars(3, 4, 5, 8, 9, 10, 13, 14, 15, 18, 19, 20), 18);

  // All Tracks also holds Open Key 1m: the same pick takes it.
  await allTracks(page);
  await expect.poll(() => listed(page)).toBe(17);
  await expect(count(page)).toHaveText("17 of 30 tracks");
  await expect(control(page, "Key 8A")).toBeVisible();

  // Searching down to tracks of one spelling, the pick still means 8A, and
  // stays offered (and pressed) to be undone.
  await bar(page).getByRole("textbox", { name: "Filter text" }).fill("Bar 28");
  await shows(page, bars(28), 30);
  await control(page, "Key 8A").click();
  const eight = panelGroup(page, "Keys").getByRole("button", { name: "8A", exact: true });
  await expect(eight).toHaveAttribute("aria-pressed", "true");
  await eight.click();
  await bar(page).getByRole("textbox", { name: "Filter text" }).fill("");
  await expect.poll(() => listed(page)).toBe(30);

  // A key off the wheel is matched exactly.
  await pick(page, "Key", "Keys", "Mystery");
  await shows(page, bars(29), 30);
});

test("BPM ranges are whole BPMs as the list rounds them, edges included", async ({ page }) => {
  await open(page);
  await bpm(page, "121", "122");
  // 120.50 (Bar 02) to 122.25 (Bar 09); 120.25 rounds to 120, 122.50 to 123.
  await shows(page, bars(2, 3, 4, 5, 6, 7, 8, 9), 30);
  await bpm(page, "", "120");
  await shows(page, bars(0, 1), 30);
});

test("MyTags: All and Any within a category, every category together", async ({ page }) => {
  await open(page);
  await pick(page, "Situation", "Situation tags", "Peak");
  await pick(page, "Situation", "Situation tags", "Warm-up");
  await shows(page, bars(0, 10, 20), 30);
  await panelGroup(page, "Situation match").getByRole("button", { name: "Any" }).click();
  await shows(page, bars(0, 2, 4, 5, 6, 8, 10, 12, 14, 15, 16, 18, 20, 22, 24, 25, 26, 28), 30);
  await pick(page, "Components", "Components tags", "Vocal");
  await shows(page, bars(0, 14, 28), 30);
});

test("a category's Any chosen before its tags holds, and survives emptying it", async ({ page }) => {
  await open(page);
  await control(page, "Components").click();
  await panelGroup(page, "Components match").getByRole("button", { name: "Any" }).click();
  await panelGroup(page, "Components tags").getByRole("button", { name: "Synth" }).click();
  await panelGroup(page, "Components tags").getByRole("button", { name: "Vocal" }).click();
  await shows(page, bars(0, 4, 7, 8, 12, 14, 16, 20, 21, 24, 28), 30);
  await panelGroup(page, "Components tags").getByRole("button", { name: "Synth" }).click();
  await panelGroup(page, "Components tags").getByRole("button", { name: "Vocal" }).click();
  await expect(panelGroup(page, "Components match").getByRole("button", { name: "Any" })).toHaveAttribute("aria-pressed", "true");
});

test("an intelligent playlist is narrowed and counted out of its own tracks", async ({ page }) => {
  await open(page);
  await shellPlaylist(page, "Fresh 128s").click();
  await expect(page.getByTestId("browser-title")).toContainText("Fresh 128s");
  await expect(count(page)).toHaveText("10 tracks");
  await energy(page, "1", "5");
  await shows(page, bars(4, 7, 10, 16, 19, 22, 28), 10);
});

test("text narrows together with the fields", async ({ page }) => {
  await open(page);
  await bar(page).getByRole("textbox", { name: "Filter text" }).fill("deep");
  await expect.poll(() => listed(page)).toBe(10);
  await energy(page, "5", "5");
  await shows(page, bars(10, 22), 30);
});

test("Key match and BPM ±6% follow the loaded track, and do nothing without one", async ({ page }) => {
  await open(page);
  await control(page, "Key match").click();
  await control(page, "BPM ±6%").click();
  // No track loaded: nothing narrows and nothing is lit.
  await expect(count(page)).toHaveText("30 tracks");
  await expect(control(page, "Key match")).not.toHaveAttribute("data-active", "");

  // Bar 01 is 9A: 8A, 9A, 10A, 9B and 11A light, so every 11B and Mystery go.
  await rows(page).nth(1).locator('[data-col="title"]').dblclick();
  await expect(page.getByTestId("shell-player")).toContainText("Bar 01");
  await expect.poll(() => listed(page)).toBe(23);
  await expect(control(page, "Key match 9A")).toHaveAttribute("data-active", "");

  // Another track loaded, Bar 28 (Open Key 1m = 8A, 127.00): BPM ±6% is
  // 119.38 to 134.62 and Key match 7A, 8A, 9A, 8B, 10A. On the Stick's "Now
  // Playing Test" (3A 128, 4A 132, 8A 138, 6A 130 BPM), tempo keeps three,
  // key keeps Lost Without You, both together keep nothing.
  await control(page, "Key match 9A").click();
  await bar(page).getByRole("textbox", { name: "Filter text" }).fill("Bar 28");
  await shows(page, bars(28), 30);
  await rows(page).first().locator('[data-col="title"]').dblclick();
  await expect(page.getByTestId("shell-player")).toContainText("Bar 28");
  await bar(page).getByRole("textbox", { name: "Filter text" }).fill("");
  await tree(page).getByRole("group", { name: "Stick TEST" }).getByRole("treeitem", { name: "Now Playing Test" }).click();
  await expect(page.getByTestId("browser-title")).toContainText("Now Playing Test");
  await expect(count(page)).toHaveText("3 of 4 tracks");
  await expect(rows(page).locator('[data-col="title"]')).toHaveText(["B2ME (feat. JordinLaine) (Original Mix)", "Spectrum (Extended Mix)", "Gravity"]);
  await control(page, "Key match").click();
  await expect(count(page)).toHaveText("0 of 4 tracks");
  await control(page, "BPM ±6%").click();
  await expect(count(page)).toHaveText("1 of 4 tracks");
  await expect(rows(page).locator('[data-col="title"]')).toHaveText(["Lost Without You"]);
});

// The Stick's own MyTags: track-filter-stick-mytags.spec.ts (wbx-45.12).
test("a Stick playlist is narrowed by the Stick's own fields", async ({ page }) => {
  await open(page);
  await tree(page).getByRole("group", { name: "Stick TEST" }).getByRole("treeitem", { name: "Now Playing Test" }).click();
  await expect(page.getByTestId("browser-title")).toContainText("Now Playing Test");
  await expect(count(page)).toHaveText("4 tracks");

  await energy(page, "5", "5");
  await expect.poll(() => listed(page)).toBe(1);
  await expect(rows(page).locator('[data-col="title"]')).toHaveText(["Spectrum (Extended Mix)"]);
  await expect(count(page)).toHaveText("1 of 4 tracks");
  await energy(page, "");
  await bar(page).getByRole("button", { name: "Unrated" }).click();
  await expect(rows(page).locator('[data-col="title"]')).toHaveText(["B2ME (feat. JordinLaine) (Original Mix)", "Lost Without You"]);
  // Choices come from the Stick: its four keys, not the laptop's.
  await control(page, "Key").click();
  await expect(panelGroup(page, "Keys").getByRole("button")).toHaveText(["3A", "4A", "6A", "8A"]);
  await panelGroup(page, "Keys").getByRole("button", { name: "8A" }).click();
  await expect(rows(page).locator('[data-col="title"]')).toHaveText(["Lost Without You"]);
});

test("selected tracks the filter hides are deselected, said, and never acted on", async ({ page }) => {
  await open(page);
  await rows(page).first().locator('[data-col="title"]').click();
  await page.keyboard.press("Control+a");
  await expect(working(page).locator('[role="row"][aria-selected="true"]')).not.toHaveCount(0);

  await energy(page, "5", "5");
  await shows(page, bars(10, 11, 22, 23), 30);
  await expect(bar(page).getByRole("note")).toHaveText("26 selected tracks the filter hides were deselected");

  // Add the selection to the empty "Eurodance": only the four shown go.
  await rows(page).first().locator('[data-col="title"]').click({ button: "right" });
  await page.getByRole("menu", { name: "Track" }).getByRole("menuitem", { name: "Add To Playlist" }).hover();
  await page.getByRole("menuitem", { name: "Eurodance" }).first().click();
  await bar(page).getByRole("button", { name: "Clear filter" }).click();
  await shellPlaylist(page, "Eurodance").click();
  await expect(page.getByTestId("browser-title")).toContainText("Eurodance");
  await expect.poll(() => listed(page)).toBe(4);
  await expect(rows(page).locator('[data-col="title"]')).toHaveText(bars(10, 11, 22, 23));
});

test("clearing restores the whole list", async ({ page }) => {
  await open(page);
  await pick(page, "Situation", "Situation tags", "Peak");
  await energy(page, "3", "5");
  await expect.poll(() => listed(page)).toBeLessThan(30);
  await bar(page).getByRole("button", { name: "Clear filter" }).click();
  await expect.poll(() => listed(page)).toBe(30);
  await expect(count(page)).toHaveText("30 tracks");
  await expect(control(page, "Situation")).not.toHaveAttribute("data-active", "");
});

test("the controls work from the keyboard, and Space and Enter never reach the deck", async ({ page }) => {
  await open(page);
  const deck = page.getByTestId("shell-player");
  await rows(page).nth(3).locator('[data-col="title"]').dblclick();
  await expect(deck.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  const energyControl = control(page, "Energy");
  await energyControl.focus();
  await page.keyboard.press("Enter");
  await expect(energyControl).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("Escape");
  await expect(energyControl).toHaveAttribute("aria-expanded", "false");
  await expect(energyControl).toBeFocused();

  await control(page, "Situation").focus();
  await page.keyboard.press("Space");
  const peak = panelGroup(page, "Situation tags").getByRole("button", { name: "Peak" });
  await peak.focus();
  await page.keyboard.press("Space");
  await expect(peak).toHaveAttribute("aria-pressed", "true");
  await control(page, "Key match").focus();
  await page.keyboard.press("Enter");
  await expect(control(page, "Key match")).toHaveAttribute("aria-pressed", "true");
  await expect.poll(() => listed(page)).toBeLessThan(30);
  await expect(deck.getByRole("button", { name: "Play", exact: true })).toBeVisible();
  await expect(deck.getByRole("button", { name: "Pause", exact: true })).toHaveCount(0);
});

test("a count that cannot be read says so instead of showing an old number", async ({ page }) => {
  await open(page);
  await expect(count(page)).toHaveText("30 tracks");
  await page.evaluate(() => {
    (window as unknown as { __failOpenView?: boolean }).__failOpenView = true;
  });
  await energy(page, "4", "5");
  await expect(count(page)).toHaveText(/count unavailable/i);
});

/** The Key panel offers exactly these choices, in this order. */
async function offersKeys(page: Page, expected: string[]) {
  await control(page, "Key").click();
  await expect(panelGroup(page, "Keys").getByRole("button")).toHaveText(expected);
}

test("keys are offered as the list writes them: Camelot, in wheel order", async ({ page }) => {
  await open(page);
  await offersKeys(page, ["8A", "9A", "11B", "Mystery"]);
});

test("with Classic keys chosen, keys are offered in Classic, alphabetically", async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
    advanced: { protectLibrary: false },
    view: { viewShell: true, keyDisplay: "classic" },
  })));
  await page.goto("/?writable=1&filterFixture=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await allTracks(page);
  await offersKeys(page, ["A", "Am", "Em", "Mystery"]);
});

test("at half width the bar stays within the window and does not push the list's search away", async ({ page }) => {
  await open(page);
  // The tree gives way at half width; All Tracks is already open.
  await page.setViewportSize({ width: 640, height: 800 });
  await energy(page, "3", "5");
  await expect(count(page)).toHaveText(/of 30 tracks/);
  for (const part of [count(page), bar(page).getByRole("button", { name: "Clear filter" }), working(page).getByRole("searchbox")] as Locator[]) {
    const box = await part.boundingBox();
    expect(box, "drawn").not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(640);
  }
});
