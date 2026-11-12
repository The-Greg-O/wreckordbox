/**
 * The view shell (ticket #50): one frame, four views.
 *
 * Behind a switch that is off unless turned on (`view.viewShell` in the
 * stored preferences), the window draws a top bar with four view tabs
 * (Library, Intake, Review, Stick), a tree area, a player area, a working
 * area and a status line. Only the working area changes between views, so
 * playback, the loaded track and the tree's selection survive a switch.
 * With the switch off the window is upstream's layout, untouched: every
 * other spec in this folder runs that way.
 */
import { expect, test, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const tabs = (page: Page) => page.getByRole("tablist", { name: "Views" });
const tab = (page: Page, name: string) => tabs(page).getByRole("tab", { name });
const working = (page: Page) => page.getByRole("tabpanel");
const titles = (page: Page) => page.locator('[role="gridcell"][data-col="title"]');
/** A playlist in the tree's Playlists group (a Stick can hold one of the same name). */
const shellPlaylist = (page: Page, name: string) =>
  page.getByTestId("shell-tree").getByRole("group", { name: "Playlists" }).getByRole("treeitem", { name });

async function open(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
}

test("with the switch off the window is the old layout, with no view tabs", async ({ page }) => {
  await open(page);
  await expect(tabs(page)).toHaveCount(0);
  await expect(page.getByTestId("shell")).toHaveCount(0);
  // Upstream's frame: its body row with the tree splitter, and its layout menu.
  await expect(page.getByTestId("body")).toBeVisible();
  await expect(page.getByRole("separator", { name: "Resize the library tree" })).toBeVisible();
});

test("with the switch on the window shows the frame: view tabs, tree, player, working area and status line", async ({ page }) => {
  await enableViewShell(page);
  await open(page);

  await expect(tab(page, "Library")).toBeVisible();
  await expect(tab(page, "Intake")).toBeVisible();
  await expect(tab(page, "Review")).toBeVisible();
  await expect(tab(page, "Stick")).toBeVisible();
  await expect(tabs(page).getByRole("tab")).toHaveCount(4);
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");

  await expect(page.getByTestId("shell-tree").getByRole("tree")).toBeVisible();
  await expect(page.getByTestId("shell-player").getByRole("region", { name: "Preview player" })).toBeVisible();
  await expect(working(page)).toBeVisible();
  await expect(page.getByTestId("shell-status")).toBeVisible();
  // The Library view lists tracks with the existing list.
  await expect(working(page).getByRole("grid")).toBeVisible();
  expect(await working(page).locator('[role="gridcell"][data-col="title"]').count()).toBeGreaterThan(5);
});

test("clicking a tab switches the working area, and the other three views are labelled empty states", async ({ page }) => {
  await enableViewShell(page);
  await open(page);

  for (const name of ["Intake", "Review", "Stick"]) {
    await tab(page, name).click();
    await expect(tab(page, name)).toHaveAttribute("aria-selected", "true");
    await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "false");
    await expect(working(page).getByRole("heading", { name })).toBeVisible();
    await expect(working(page)).toContainText("Nothing is here yet");
    // Only the working area changed: no track list in it now.
    await expect(working(page).getByRole("grid")).toHaveCount(0);
    // The rest of the frame is still there.
    await expect(page.getByTestId("shell-tree").getByRole("tree")).toBeVisible();
    await expect(page.getByTestId("shell-player").getByRole("region", { name: "Preview player" })).toBeVisible();
    await expect(page.getByTestId("shell-status")).toBeVisible();
  }

  await tab(page, "Library").click();
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
  await expect(working(page).getByRole("grid")).toBeVisible();
  await expect(titles(page).first()).toBeVisible();
});

test("keys 1 to 4 switch views", async ({ page }) => {
  await enableViewShell(page);
  await open(page);

  await page.keyboard.press("2");
  await expect(tab(page, "Intake")).toHaveAttribute("aria-selected", "true");
  await expect(working(page).getByRole("heading", { name: "Intake" })).toBeVisible();
  await page.keyboard.press("3");
  await expect(tab(page, "Review")).toHaveAttribute("aria-selected", "true");
  await expect(working(page).getByRole("heading", { name: "Review" })).toBeVisible();
  await page.keyboard.press("4");
  await expect(tab(page, "Stick")).toHaveAttribute("aria-selected", "true");
  await expect(working(page).getByRole("heading", { name: "Stick" })).toBeVisible();
  await page.keyboard.press("1");
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
  await expect(working(page).getByRole("grid")).toBeVisible();
});

const player = (page: Page) => page.getByRole("region", { name: "Preview player" });
// The shell's player area holds the compact player (#51).
const deckTitle = (page: Page) => player(page).getByTestId("compact-player-title");
/** A loop lit on the deck's waveform. */
const activeLoop = (page: Page) => page.locator('[class*="loopBand"][data-active]');
/** Where the playhead is across the overview, in pixels: it moves while the deck runs. */
async function playhead(page: Page): Promise<number> {
  return (await player(page).getByTestId("compact-player-head").boundingBox())?.x ?? Number.NaN;
}

test("switching views does not stop playback, change the loaded track or lose the tree selection", async ({ page }) => {
  await enableViewShell(page);
  await open(page);

  // A playlist selected in the tree, one of its tracks loaded and playing.
  const playlist = shellPlaylist(page, "Melodic Vox");
  await playlist.click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  const loaded = await titles(page).nth(1).innerText();
  await titles(page).nth(1).dblclick();
  await expect(deckTitle(page)).toContainText(loaded);
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeEnabled();
  await player(page).getByRole("button", { name: "Play", exact: true }).click();
  await expect(player(page).getByRole("button", { name: "Pause", exact: true })).toBeVisible();

  // Through every other view, by key and by click.
  for (const [name, key] of [["Intake", "2"], ["Review", "3"], ["Stick", null]] as const) {
    const before = await playhead(page);
    if (key !== null) await page.keyboard.press(key);
    else await tab(page, name).click();
    await expect(working(page).getByRole("heading", { name })).toBeVisible();
    await expect(deckTitle(page)).toContainText(loaded);
    await expect(player(page).getByRole("button", { name: "Pause", exact: true })).toBeVisible();
    await expect(playlist).toHaveAttribute("aria-selected", "true");
    // Still running, not merely showing a pause button: the position moves on.
    await expect.poll(() => playhead(page), { timeout: 10_000 }).toBeGreaterThan(before);
  }

  // Back in the Library the list is still the selected playlist's.
  await page.keyboard.press("1");
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await expect(playlist).toHaveAttribute("aria-selected", "true");
  await expect(deckTitle(page)).toContainText(loaded);
  await expect(player(page).getByRole("button", { name: "Pause", exact: true })).toBeVisible();
});

test("a digit typed into the search field is text, not a view switch", async ({ page }) => {
  await enableViewShell(page);
  await open(page);

  const search = working(page).getByRole("searchbox");
  await search.click();
  await page.keyboard.type("2");
  await expect(search).toHaveValue("2");
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
  await page.keyboard.type("34");
  await expect(search).toHaveValue("234");
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
});

test("keys 1 to 4 are the views' and no longer set a hot cue or a beat loop on the deck", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  const cues = player(page).locator("[data-cue]");

  // The player draws a track's hot cues: the first row has some.
  await titles(page).nth(0).dblclick();
  await expect.poll(() => cues.count()).toBeGreaterThan(0);
  // The third row has none, so a hot cue set by a key would show.
  const bare = await titles(page).nth(2).innerText();
  await titles(page).nth(2).dblclick();
  await expect(deckTitle(page)).toContainText(bare);
  await expect(cues).toHaveCount(0);

  for (const key of ["1", "2", "3", "4", "1"]) await page.keyboard.press(key);
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
  await expect(deckTitle(page)).toContainText(bare);
  // Give a cue write the time the old layout's takes to come back and draw.
  await page.waitForTimeout(1000);
  await expect(cues).toHaveCount(0);

  // Nor does 4 start the deck's 1 Beat Loop. The first row has a beat grid,
  // so a loop can be set on it (with the switch off, 4 lights one).
  const gridded = await titles(page).nth(0).innerText();
  await titles(page).nth(0).dblclick();
  await expect(deckTitle(page)).toContainText(gridded);
  await expect.poll(() => cues.count()).toBeGreaterThan(0);
  await page.keyboard.press("4");
  await expect(tab(page, "Stick")).toHaveAttribute("aria-selected", "true");
  await page.waitForTimeout(1000);
  await expect(activeLoop(page)).toHaveCount(0);
  await expect(player(page).getByRole("button", { name: "Exit loop" })).toHaveCount(0);
});

const rows = (page: Page) => page.getByRole("row").filter({ has: page.getByRole("gridcell") });
const selectedRows = (page: Page) => rows(page).and(page.locator("[data-selected]"));
const analysis = (page: Page) => page.getByRole("dialog", { name: "Analysis Setting" });

test("what Analyze targets is what the list shows as selected, before and after a view switch", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");

  await rows(page).nth(2).click();
  await rows(page).nth(4).click({ modifiers: ["Shift"] });
  await expect(selectedRows(page)).toHaveCount(3);
  await page.keyboard.press("Control+Shift+A");
  await expect(analysis(page)).toContainText("3 tracks selected");
  await analysis(page).getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(analysis(page)).toHaveCount(0);

  // Away and back: the list comes back with nothing selected, so Analyze
  // has nothing to act on. It must not reach the three chosen before.
  await page.keyboard.press("2");
  await expect(working(page).getByRole("heading", { name: "Intake" })).toBeVisible();
  // Nor from another view, where no list is shown at all.
  await page.keyboard.press("Control+Shift+A");
  await page.waitForTimeout(500);
  await expect(analysis(page)).toHaveCount(0);
  await page.keyboard.press("1");
  await expect(titles(page).first()).toBeVisible();
  await expect(selectedRows(page)).toHaveCount(0);
  await page.keyboard.press("Control+Shift+A");
  await page.waitForTimeout(500);
  await expect(analysis(page)).toHaveCount(0);

  // A fresh selection is what Analyze takes.
  await rows(page).nth(1).click();
  await expect(selectedRows(page)).toHaveCount(1);
  await page.keyboard.press("Control+Shift+A");
  await expect(analysis(page)).toContainText("1 track selected");
});

test("a view key pressed in a confirmation prompt leaves the view alone", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  // A track added to the playlist that already holds it: the window asks
  // whether to add it again.
  await shellPlaylist(page, "Melodic Vox").click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await titles(page).first().click({ button: "right" });
  await page.getByRole("menu", { name: "Track" }).getByRole("menuitem", { name: "Add To Playlist" }).hover();
  await page.getByRole("menuitem", { name: "Melodic Vox" }).first().click();
  const question = page.getByRole("alertdialog", { name: "Duplicate tracks" });
  await expect(question).toBeVisible();

  // The prompt has the keyboard: one of its buttons is focused.
  await question.getByRole("button").first().focus();
  await page.keyboard.press("2");
  await expect(question).toBeVisible();
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
  await expect(tab(page, "Intake")).toHaveAttribute("aria-selected", "false");
});

test("the layout follows the window's own width: the tree gives way at 800 and below, and nothing overlaps", async ({ page }) => {
  await enableViewShell(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  await open(page);

  const tree = page.getByTestId("shell-tree");
  const box = async (id: string) => {
    const found = await page.getByTestId(id).boundingBox();
    if (!found) throw new Error(`${id} is not drawn`);
    return found;
  };
  /** The frame's areas tile the window: each inside it, none over another. */
  async function expectNoOverlap(width: number, areas: string[]) {
    const boxes = await Promise.all(areas.map(box));
    for (const [index, area] of boxes.entries()) {
      expect(area.x, areas[index]).toBeGreaterThanOrEqual(0);
      expect(area.x + area.width, areas[index]).toBeLessThanOrEqual(width + 0.5);
      expect(area.y + area.height, areas[index]).toBeLessThanOrEqual(800 + 0.5);
      expect(area.width, areas[index]).toBeGreaterThan(0);
      expect(area.height, areas[index]).toBeGreaterThan(0);
      for (const [otherIndex, other] of boxes.entries()) {
        if (otherIndex <= index) continue;
        const apart = area.x + area.width <= other.x + 0.5 || other.x + other.width <= area.x + 0.5
          || area.y + area.height <= other.y + 0.5 || other.y + other.height <= area.y + 0.5;
        expect(apart, `${areas[index]} and ${areas[otherIndex]} overlap at ${width}`).toBe(true);
      }
    }
  }

  // Full width: the tree beside the working area.
  await expect(tree).toBeVisible();
  await expectNoOverlap(1280, ["shell-top-bar", "shell-player", "shell-tree", "shell-working", "shell-status"]);
  expect((await box("shell-working")).x).toBeGreaterThan((await box("shell-tree")).x);

  // A playlist chosen in the tree, to see that it survives the tree giving way.
  const playlist = shellPlaylist(page, "Melodic Vox");
  await playlist.click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");

  // Just above the breakpoint the tree is still there; at it, it has gone.
  await page.setViewportSize({ width: 801, height: 800 });
  await expect(tree).toBeVisible();
  await page.setViewportSize({ width: 800, height: 800 });
  await expect(tree).toBeHidden();

  // Half width: tabs, player, list and status line, the list the full width.
  await page.setViewportSize({ width: 640, height: 800 });
  await expect(tree).toBeHidden();
  await expect(tabs(page).getByRole("tab")).toHaveCount(4);
  for (const name of ["Library", "Intake", "Review", "Stick"]) await expect(tab(page, name)).toBeVisible();
  await expectNoOverlap(640, ["shell-top-bar", "shell-player", "shell-working", "shell-status"]);
  expect((await box("shell-working")).width).toBeCloseTo(640, 0);
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await expect(titles(page).first()).toBeVisible();

  // Narrow: still nothing over anything else.
  await page.setViewportSize({ width: 436, height: 800 });
  await expectNoOverlap(436, ["shell-top-bar", "shell-player", "shell-working", "shell-status"]);
  for (const name of ["Library", "Intake", "Review", "Stick"]) await expect(tab(page, name)).toBeVisible();

  // Wide again: the tree comes back with its selection.
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(tree).toBeVisible();
  await expect(playlist).toHaveAttribute("aria-selected", "true");
});
