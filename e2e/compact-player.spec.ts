/**
 * The compact player (ticket #51): the same strip in every view of the view
 * shell. Cue and play, the track's title and one line of artist, tempo, key
 * and position, the overview waveform with its hot cue markers and a phrase
 * strip under it, and beat jump back and forward by 4, 8, 16 or 32 beats
 * with the size on a selector.
 *
 * It is the shell's player, so every test here turns the shell on; with the
 * switch off the window keeps upstream's deck, which the other specs drive.
 */
import { expect, test, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const player = (page: Page) => page.getByTestId("shell-player").getByRole("region", { name: "Preview player" });
const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const titles = (page: Page) => page.locator('[role="gridcell"][data-col="title"]');
const overview = (page: Page) => player(page).getByTestId("compact-player-overview");
const phraseStrip = (page: Page) => player(page).getByTestId("compact-player-phrases");
const sizes = (page: Page) => player(page).getByRole("radiogroup", { name: "Beat jump size" });
const size = (page: Page, beats: number) => sizes(page).getByRole("radio", { name: `${beats} beats`, exact: true });
const jumpBack = (page: Page) => player(page).getByRole("button", { name: "Jump back", exact: true });
const jumpForward = (page: Page) => player(page).getByRole("button", { name: "Jump forward", exact: true });

/** The position readout's elapsed part in seconds: `1:23.4 / 5:48` is 83.4. */
async function elapsedSeconds(page: Page): Promise<number> {
  const text = (await player(page).getByTestId("compact-player-elapsed").textContent()) ?? "";
  const [, minutes, seconds] = /(\d+):(\d+(?:\.\d)?)/.exec(text) ?? [];
  return Number(minutes) * 60 + Number(seconds);
}
/** How long `beats` last at the loaded track's tempo, from the tempo readout. */
async function secondsOf(page: Page, beats: number): Promise<number> {
  const readout = player(page).getByTestId("compact-player-tempo");
  await expect(readout).toHaveText(/^[1-9]\d*\.\d\d$/);
  return (beats * 60) / Number(await readout.textContent());
}
/** The readout shows tenths, so a landing is right to within one of them. */
async function expectHeadAt(page: Page, seconds: number) {
  await expect.poll(() => elapsedSeconds(page)).toBeCloseTo(seconds, 0);
  expect(Math.abs((await elapsedSeconds(page)) - seconds)).toBeLessThanOrEqual(0.11);
}

async function open(page: Page) {
  await enableViewShell(page);
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
}
/** Loads the fourth row: analysed, so it has a tempo, a key, hot cues and phrases. */
async function load(page: Page) {
  await open(page);
  await titles(page).nth(3).dblclick();
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeEnabled();
}

test("the compact player shows cue, play, the overview with hot cue markers, position, tempo and key in all four views", async ({ page }) => {
  await open(page);
  const title = await titles(page).nth(3).innerText();
  await titles(page).nth(3).dblclick();

  for (const view of ["Library", "Intake", "Review", "Stick"]) {
    await tab(page, view).click();
    await expect(tab(page, view)).toHaveAttribute("aria-selected", "true");
    await expect(player(page).getByRole("button", { name: "Cue", exact: true })).toBeVisible();
    await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();
    await expect(player(page).getByTestId("compact-player-title")).toHaveText(title);
    await expect(overview(page)).toBeVisible();
    // The marker is a zero-width line; its lettered badge is what shows.
    await expect(overview(page).locator('[data-cue="A"] b')).toHaveText("A");
    await expect(overview(page).locator('[data-cue="A"] b')).toBeVisible();
    await expect(player(page).getByTestId("compact-player-position")).toHaveText(/^0:00\.0 \/ \d+:\d\d$/);
    await expect(player(page).getByTestId("compact-player-tempo")).toHaveText(/^1\d\d\.\d\d$/);
    await expect(player(page).getByTestId("compact-player-key")).toHaveText(/^\d{1,2}[AB]$|^[A-G][#b]?m?$/);
  }
});

test("a phrase strip runs under the overview for a track with phrase analysis, and is absent for one without", async ({ page }) => {
  await load(page);
  await expect(phraseStrip(page)).toBeVisible();
  await expect(phraseStrip(page).locator("[data-phrase]").first()).toBeVisible();

  // An unanalysed row has no tempo, so its BPM cell is empty, and no phrases.
  const bpms = page.locator('[role="gridcell"][data-col="bpm"]');
  const count = await bpms.count();
  let bare = -1;
  for (let i = 0; i < count; i += 1) {
    if ((await bpms.nth(i).innerText()).trim() === "") { bare = i; break; }
  }
  expect(bare, "the mock list has an unanalysed row").toBeGreaterThanOrEqual(0);
  const title = await titles(page).nth(bare).innerText();
  await titles(page).nth(bare).dblclick();
  await expect(player(page).getByTestId("compact-player-title")).toHaveText(title);
  await expect(overview(page)).toBeVisible();
  await expect(phraseStrip(page)).toHaveCount(0);
});

test("the size selector offers 4, 8, 16 and 32 beats, one at a time", async ({ page }) => {
  await load(page);
  await expect(sizes(page).getByRole("radio")).toHaveText(["4", "8", "16", "32"]);
  await expect(sizes(page).getByRole("radio", { checked: true })).toHaveCount(1);
  await size(page, 8).click();
  await expect(size(page, 8)).toHaveAttribute("aria-checked", "true");
  await expect(size(page, 16)).toHaveAttribute("aria-checked", "false");
});

test("beat jump by pointer moves forward and back by the selected size", async ({ page }) => {
  await load(page);
  const start = await elapsedSeconds(page);

  await size(page, 16).click();
  const sixteen = await secondsOf(page, 16);
  await jumpForward(page).click();
  await expectHeadAt(page, start + sixteen);
  await jumpForward(page).click();
  await expectHeadAt(page, start + sixteen * 2);

  await size(page, 4).click();
  const four = await secondsOf(page, 4);
  await jumpBack(page).click();
  await expectHeadAt(page, start + sixteen * 2 - four);

  await size(page, 32).click();
  const thirtyTwo = await secondsOf(page, 32);
  await jumpForward(page).click();
  await expectHeadAt(page, start + sixteen * 2 - four + thirtyTwo);
  await jumpBack(page).click();
  await expectHeadAt(page, start + sixteen * 2 - four);
});

test("beat jump by key: the arrows move the clicked player by the selected size", async ({ page }) => {
  await load(page);
  await size(page, 8).click();
  // A click on the player gives it the arrows, as it does the old deck.
  await player(page).getByTestId("compact-player-title").click();
  await expect(player(page)).toHaveAttribute("data-armed", "");
  const start = await elapsedSeconds(page);
  const eight = await secondsOf(page, 8);

  await page.keyboard.press("ArrowRight");
  await expectHeadAt(page, start + eight);
  await page.keyboard.press("ArrowRight");
  await expectHeadAt(page, start + eight * 2);
  await page.keyboard.press("ArrowLeft");
  await expectHeadAt(page, start + eight);
});

test("the chosen size persists into the next session", async ({ page, context }) => {
  await load(page);
  await size(page, 32).click();
  await expect(size(page, 32)).toHaveAttribute("aria-checked", "true");

  // A second page in the same profile, without the seeding script, reads
  // what the first one stored.
  const next = await context.newPage();
  await next.goto("/");
  await expect(next.getByTestId("browser-title")).toContainText("Tracks)");
  await next.locator('[role="gridcell"][data-col="title"]').nth(3).dblclick();
  await expect(size(next, 32)).toHaveAttribute("aria-checked", "true");
  await expect(size(next, 4)).toHaveAttribute("aria-checked", "false");

  // The size the key uses is the stored one too.
  await player(next).getByTestId("compact-player-title").click();
  const start = await elapsedSeconds(next);
  const thirtyTwo = await secondsOf(next, 32);
  await next.keyboard.press("ArrowRight");
  await expectHeadAt(next, start + thirtyTwo);
});
