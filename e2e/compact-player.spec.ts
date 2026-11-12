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
import {
  FOURTH_ROW_ID, elapsedSeconds, engine, engineMovedFrom, expectHeadAt, jumpBack, jumpForward, load, open, overview,
  pauseButton, playButton, player, secondsOf, seekTo, size, sizes, tempo, titles,
} from "./compactPlayer";

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const phraseStrip = (page: Page) => player(page).getByTestId("compact-player-phrases");
const cueButton = (page: Page) => player(page).getByRole("button", { name: "Cue", exact: true });
const loadedTitle = (page: Page) => player(page).getByTestId("compact-player-title");

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

test("the arrows jump only while the player has the keyboard: not before it is clicked, nor after the list or the tree is", async ({ page }) => {
  await load(page);
  await size(page, 8).click();
  // The size was clicked, so the player has the keyboard; give it back to
  // the list by clicking a row, as a DJ browsing does.
  await titles(page).nth(5).click();
  await expect(player(page)).not.toHaveAttribute("data-armed", "");
  const start = (await engine(page)).a;

  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowRight");
  await page.waitForTimeout(400);
  expect((await engine(page)).a).toBeCloseTo(start, 3);

  // Clicked: the arrows are the player's.
  await loadedTitle(page).click();
  await expect(player(page)).toHaveAttribute("data-armed", "");
  await page.keyboard.press("ArrowRight");
  const moved = await engineMovedFrom(page, start);
  expect(moved - start).toBeCloseTo(await secondsOf(page, 8), 1);

  // The tree takes the keyboard back.
  await page.getByTestId("shell-tree").click({ position: { x: 20, y: 8 } });
  await expect(player(page)).not.toHaveAttribute("data-armed", "");
  await page.keyboard.press("ArrowRight");
  await page.waitForTimeout(400);
  expect((await engine(page)).a).toBeCloseTo(moved, 3);
});

test("Space and Enter on the size selector and the jump buttons press those controls, and never play, pause or load", async ({ page }) => {
  await load(page);
  const title = await loadedTitle(page).innerText();
  // Another row selected, so Enter as the deck's Load would replace the track.
  await titles(page).nth(6).click();
  const other = await titles(page).nth(6).innerText();
  expect(other).not.toBe(title);

  await size(page, 4).focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Enter");
  await expect(size(page, 4)).toHaveAttribute("aria-checked", "true");
  await page.waitForTimeout(300);
  await expect(playButton(page)).toBeVisible();
  await expect(loadedTitle(page)).toHaveText(title);

  const start = (await engine(page)).a;
  const four = await secondsOf(page, 4);
  await jumpForward(page).focus();
  await page.keyboard.press("Enter");
  const once = await engineMovedFrom(page, start);
  expect(once - start).toBeCloseTo(four, 1);
  await page.keyboard.press("Space");
  const twice = await engineMovedFrom(page, once);
  expect(twice - start).toBeCloseTo(four * 2, 1);
  await jumpBack(page).focus();
  await page.keyboard.press("Space");
  await engineMovedFrom(page, twice);
  await page.waitForTimeout(300);
  await expect(playButton(page)).toBeVisible();
  await expect(loadedTitle(page)).toHaveText(title);
});

test("CUE answers the keyboard as it does the pointer: held, it previews from the cue point; let go, it returns there", async ({ page }) => {
  await load(page);
  // Away from the track's cue point and paused, a press of CUE sets the cue
  // point at the head (snapped to the nearest beat, as a click of CUE does)
  // and does not play: the same press by key, then a hold.
  await seekTo(page, 0.3);
  await expect.poll(async () => (await engine(page)).a).toBeGreaterThan(1);
  const clicked = (await engine(page)).a;
  const beat = await secondsOf(page, 1);
  await cueButton(page).focus();
  await page.keyboard.press("Space");
  await page.waitForTimeout(300);
  await expect(playButton(page)).toBeVisible();
  const cuePoint = (await engine(page)).a;
  expect(Math.abs(cuePoint - clicked)).toBeLessThanOrEqual(beat / 2 + 0.01);

  await page.keyboard.down("Space");
  await expect(pauseButton(page)).toBeVisible();
  await expect.poll(async () => (await engine(page)).aNow - cuePoint).toBeGreaterThan(0.2);
  await page.keyboard.up("Space");
  await expect(playButton(page)).toBeVisible();
  await expect.poll(async () => Math.abs((await engine(page)).a - cuePoint)).toBeLessThan(0.05);

  // Enter does the same.
  await page.keyboard.down("Enter");
  await expect(pauseButton(page)).toBeVisible();
  await page.keyboard.up("Enter");
  await expect(playButton(page)).toBeVisible();
  await expect.poll(async () => Math.abs((await engine(page)).a - cuePoint)).toBeLessThan(0.05);
});

test("a jump back from the start shows the pre-roll as a signed position, as the engine has it", async ({ page }) => {
  await load(page);
  await seekTo(page, 0);
  await expect.poll(async () => (await engine(page)).a).toBeLessThan(0.5);
  const start = (await engine(page)).a;
  await size(page, 4).click();
  const four = await secondsOf(page, 4);
  await jumpBack(page).click();
  const landed = await engineMovedFrom(page, start);
  expect(landed).toBeLessThan(0);
  expect(landed).toBeCloseTo(start - four, 1);
  await expect(player(page).getByTestId("compact-player-elapsed")).toHaveText(/^-0:0\d\.\d$/);
  await expectHeadAt(page, landed);
});

test("the tempo readout is the grid's tempo under the playhead, across a tempo change", async ({ page }) => {
  // `?grids=<id>:double`: the fourth row's second half runs at twice its tempo.
  await load(page, `?grids=${FOURTH_ROW_ID}:double`);
  await seekTo(page, 0.1);
  const slow = await tempo(page);
  await seekTo(page, 0.8);
  await expect.poll(() => tempo(page)).toBeCloseTo(slow * 2, 1);
  await seekTo(page, 0.25);
  await expect.poll(() => tempo(page)).toBeCloseTo(slow, 1);
});

for (const width of [640, 436]) {
  for (const scale of [{ stop: 4, label: "100%" }, { stop: 6, label: "125%" }, { stop: 2, label: "80%" }]) {
    test(`at ${width} px and ${scale.label} the position, tempo and key are drawn whole inside the player`, async ({ page }) => {
      await page.setViewportSize({ width, height: 600 });
      await load(page, "", 3, { uiScale: scale.stop });
      const clipped = await page.evaluate(() => {
        const strip = document.querySelector<HTMLElement>('[data-testid="compact-player"]');
        if (!strip) return ["no player"];
        const problems: string[] = [];
        for (const id of ["compact-player-position", "compact-player-tempo", "compact-player-key", "compact-player-title"]) {
          const el: HTMLElement | null = strip.querySelector<HTMLElement>(`[data-testid="${id}"]`);
          if (!el) { problems.push(`${id} missing`); continue; }
          const box = el.getBoundingClientRect();
          if (box.width <= 0 || box.height <= 0) problems.push(`${id} has no size`);
          // The title may ellipsize; the readouts may not, so they must fit
          // every box that clips them, up to the player and the window.
          if (id === "compact-player-title") continue;
          if (el.scrollWidth > el.clientWidth + 1) problems.push(`${id} overflows itself`);
          for (let up: HTMLElement | null = el.parentElement; up; up = up.parentElement) {
            const style = getComputedStyle(up);
            if (style.overflowX === "visible" && style.overflowY === "visible" && up !== strip) continue;
            const clip = up.getBoundingClientRect();
            if (box.left < clip.left - 0.5 || box.right > clip.right + 0.5 || box.top < clip.top - 0.5 || box.bottom > clip.bottom + 0.5) {
              problems.push(`${id} is cut by ${up.dataset.testid ?? up.className}`);
            }
            if (up === strip) break;
          }
          if (box.right > window.innerWidth + 0.5) problems.push(`${id} is off the window`);
        }
        return problems;
      });
      expect(clipped).toEqual([]);
      await expect(player(page).getByTestId("compact-player-tempo")).toBeVisible();
      await expect(overview(page)).toBeVisible();
      await expect(jumpForward(page)).toBeVisible();
    });
  }
}
