/**
 * The half-width breadcrumb's count and length after the playlist name
 * (wbx-6wz; docs/design/screens/mock.html "CURRENT / Melodic Vox 30 ·
 * 2h24m"). At 640 and 436 px and 0.8, 1, 1.25 it is drawn whole, inside the
 * breadcrumb, after the name; it is the open list's, never the one before;
 * a list with no tally (All Tracks) shows none; and when the room runs out
 * it gives way before the name does.
 *
 * Mock: `?filterFixture=1` ("Melodic Vox" in folder CURRENT holds 18 tracks,
 * "Eurodance" none). The expected length is the sum of the list's own Time
 * column, read at 640 where it is drawn.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { DEFAULT_PREFERENCES } from "../src/lib/preferences";

const SCALES = [{ stop: 2, factor: 0.8 }, { stop: 4, factor: 1 }, { stop: 6, factor: 1.25 }];

const working = (page: Page) => page.getByRole("tabpanel");
const crumb = (page: Page) => working(page).getByRole("button", { name: /^Open the tree/ });
const tally = (page: Page) => crumb(page).locator("[data-crumb-tally]");
const name = (page: Page) => crumb(page).locator("[data-crumb]").last();
const caret = (page: Page) => crumb(page).locator("[data-crumb-caret]");
const treePane = (page: Page) => page.getByTestId("shell-tree");
const tree = (page: Page) => page.getByRole("tree", { name: "Tree" });
const playlist = (page: Page, label: string) => tree(page).getByRole("group", { name: "Playlists" }).getByRole("treeitem", { name: label, exact: true });
const rows = (page: Page) => working(page).getByTestId("track-scroll").locator('[role="row"][aria-selected]');

async function open(page: Page, width: number, stop: number): Promise<void> {
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.addInitScript((full) => {
    if (sessionStorage.getItem("tally-seeded")) return;
    sessionStorage.setItem("tally-seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify(full));
  }, { ...DEFAULT_PREFERENCES, advanced: { ...DEFAULT_PREFERENCES.advanced, protectLibrary: false },
    view: { ...DEFAULT_PREFERENCES.view, viewShell: true, uiScale: stop } });
  await page.goto("/?writable=1&filterFixture=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  await playlist(page, "Melodic Vox").click();
  await expect(page.getByTestId("browser-title")).toContainText("Melodic Vox");
  await expect(rows(page).first()).toBeVisible();
  await page.setViewportSize({ width, height: 900 });
  await expect(treePane(page)).toBeHidden();
}

type Box = { x: number; y: number; width: number; height: number };
const box = async (l: Locator) => (await l.boundingBox())!;
const inside = (a: Box, b: Box) => a.x >= b.x - 0.5 && a.y >= b.y - 0.5 && a.x + a.width <= b.x + b.width + 0.5 && a.y + a.height <= b.y + b.height + 0.5;
/** Whether an element's text is cut short (an ellipsis or a clip). */
const cut = (l: Locator) => l.evaluate((e) => e.scrollWidth > e.clientWidth + 0.5);

/** The tally, drawn whole, inside the breadcrumb, between the name and the caret, within the window. */
async function expectTallyLaidOut(page: Page, width: number): Promise<void> {
  await expect(tally(page)).toBeVisible();
  const c = await box(crumb(page));
  expect(c.x + c.width).toBeLessThanOrEqual(width + 0.5);
  const t = await box(tally(page));
  expect(inside(t, c)).toBe(true);
  expect(await cut(tally(page))).toBe(false);
  const n = await box(name(page));
  expect(n.x + n.width).toBeLessThanOrEqual(t.x + 0.5);
  expect(t.x + t.width).toBeLessThanOrEqual((await box(caret(page))).x + 0.5);
}

/** Sum of the drawn rows' Time cells (mm:ss), as the breadcrumb words it. */
async function lengthOfRows(page: Page): Promise<string> {
  const times = await rows(page).locator('[data-col="duration"]').allTextContents();
  const seconds = times.reduce((sum, text) => {
    const [m, s] = text.trim().split(":").map(Number);
    return sum + (m ?? 0) * 60 + (s ?? 0);
  }, 0);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return hours > 0 ? `${hours}h${String(minutes).padStart(2, "0")}m` : `${minutes}m`;
}

/** A name longer than any breadcrumb here has room for. */
const LONG = "Melodic Vox and Everything Sung Over Strings Late in the Night, Second Room, Kept for the Closing Hour";

async function rename(page: Page, from: string, to: string): Promise<void> {
  await page.evaluate(([a, b]) => {
    (window as unknown as { __renamePlaylistOutside: (from: string, to: string) => void }).__renamePlaylistOutside(a!, b!);
  }, [from, to]);
}

async function choose(page: Page, label: string): Promise<void> {
  await crumb(page).click();
  await expect(treePane(page)).toBeVisible();
  const row = label === "All Tracks"
    ? tree(page).getByRole("treeitem", { name: label, exact: true }).first()
    : playlist(page, label);
  await row.click();
  await expect(treePane(page)).toBeHidden();
}

test("the count and length are the open playlist's, and only a playlist's", async ({ page }) => {
  await open(page, 640, 4);
  await expect(rows(page)).toHaveCount(18);
  const length = await lengthOfRows(page);
  await expect(tally(page)).toHaveText(`18 · ${length}`);
  // The tally is read out with the breadcrumb.
  await expect(crumb(page)).toHaveAccessibleName(`Open the tree: CURRENT / Melodic Vox, 18 · ${length}`);

  // Another playlist: its own count (an empty one says only that), never the last list's.
  await choose(page, "Eurodance");
  await expect(page.getByTestId("browser-title")).toContainText("Eurodance");
  await expect(tally(page)).toHaveText("0");

  // All Tracks has no tally: nothing is drawn after its name.
  await choose(page, "All Tracks");
  await expect(page.getByTestId("browser-title")).toContainText("All Tracks");
  await expect(tally(page)).toHaveCount(0);
  await expect(crumb(page)).toHaveAccessibleName("Open the tree: All Tracks");

  // Back to the playlist: its tally again.
  await choose(page, "Melodic Vox");
  await expect(tally(page)).toHaveText(`18 · ${length}`);
});

for (const width of [640, 436]) {
  for (const scale of SCALES) {
    test(`${width}px at ${Math.round(scale.factor * 100)}%: drawn whole after the name, and it gives way first`, async ({ page }) => {
      await open(page, width, scale.stop);
      await expect(tally(page)).toHaveText(/^18 · \d+(h\d\d)?m$/);
      await expectTallyLaidOut(page, width);
      expect(await cut(name(page))).toBe(false);

      // A name too long for both: the tally goes, the name keeps all the room up to the caret.
      await rename(page, "Melodic Vox", LONG);
      await expect(name(page)).toHaveText(LONG);
      expect(await cut(name(page))).toBe(true);
      await expect(tally(page)).toBeHidden();
      const n = await box(name(page));
      const room = (await box(caret(page))).x - (n.x + n.width);
      // Only the gaps between the parts are left: no room was kept for the tally.
      expect(room).toBeLessThan(24);

      // Short again: the tally comes back, whole.
      await rename(page, LONG, "Melodic Vox");
      await expect(name(page)).toHaveText("Melodic Vox");
      await expectTallyLaidOut(page, width);
    });
  }
}
