/**
 * Where the compact player's beat jump lands (ticket #51), read from the mock
 * engine's own head (`window.__deckSeconds`) rather than the readout: a grid
 * with a tempo change, a track with a BPM but no grid, one with neither, both
 * ends of the track, playing and paused, and with a loop running. And the
 * overview's canvas is backed at the pixels it covers, at every interface
 * scale and at narrow widths.
 *
 * `?grids=<track id>:double` gives that track's grid a second half at twice the
 * tempo; `?grids=<track id>:none` takes its grid away and leaves its BPM.
 */
import { expect, test, type Page } from "@playwright/test";
import {
  FOURTH_ROW_ID, engine, engineMovedFrom, jumpBack, jumpForward, load, open, overview, pauseButton, playButton, player,
  secondsOf, seekTo, size, tempo, titles,
} from "./compactPlayer";

/** The fourth row's length in seconds, from the readout's `/ m:ss` (whole seconds). */
async function lengthSeconds(page: Page): Promise<number> {
  const text = (await player(page).getByTestId("compact-player-position").textContent()) ?? "";
  const [, minutes, seconds] = /\/ (\d+):(\d\d)/.exec(text) ?? [];
  return Number(minutes) * 60 + Number(seconds);
}

/**
 * The fixture's grid, built here on its own terms: beats at the row's tempo
 * to half way, then at twice it. `beatsFrom` walks it from `fromS` by
 * `beats`, keeping the fraction of a beat, as a grid jump does.
 */
function doubledGrid(bpm: number, totalS: number): number[] {
  const slow = 60 / bpm;
  const times: number[] = [];
  for (let at = 0; at <= totalS; ) {
    times.push(at);
    at += at >= totalS / 2 ? slow / 2 : slow;
  }
  return times;
}
function beatsFrom(times: number[], fromS: number, beats: number): number {
  let i = 0;
  while (i < times.length - 2 && (times[i + 1] ?? 0) <= fromS) i += 1;
  const a = times[i] ?? 0;
  const b = times[i + 1] ?? a;
  const index = i + (fromS - a) / (b - a) + beats;
  const whole = Math.floor(index);
  const p = times[whole] ?? 0;
  const q = times[whole + 1] ?? p;
  return p + (index - whole) * (q - p);
}

test("on a grid with a tempo change a jump counts the grid's beats, slow side, fast side and across", async ({ page }) => {
  await load(page, `?grids=${FOURTH_ROW_ID}:double`);
  await seekTo(page, 0.1);
  const bpm = await tempo(page);
  // The readout's length is whole seconds; the mock's is the row's own.
  const total = await lengthSeconds(page);
  const grid = doubledGrid(bpm, total);
  const half = total / 2;
  await size(page, 16).click();

  // In the fast half, 16 beats last half as long as they do in the slow one.
  await seekTo(page, 0.75);
  await expect.poll(() => tempo(page)).toBeCloseTo(bpm * 2, 1);
  let from = (await engine(page)).a;
  let to = await (async () => { await jumpForward(page).click(); return engineMovedFrom(page, from); })();
  expect(to - from).toBeCloseTo((16 * 60) / (bpm * 2), 1);

  // Across the change: from a few slow beats before half way, forward 16.
  await seekTo(page, (half - 4 * (60 / bpm)) / total);
  await expect.poll(() => tempo(page)).toBeCloseTo(bpm, 1);
  from = (await engine(page)).a;
  await jumpForward(page).click();
  to = await engineMovedFrom(page, from);
  expect(to).toBeCloseTo(beatsFrom(grid, from, 16), 1);
  // A fixed-time jump at the slow tempo would land well past that.
  expect(to - from).toBeLessThan((16 * 60) / bpm - 1);

  // And back again lands where it started.
  await jumpBack(page).click();
  const back = await engineMovedFrom(page, to);
  expect(back).toBeCloseTo(from, 1);
});

test("a track with a BPM and no grid jumps by the BPM's beats", async ({ page }) => {
  await load(page, `?grids=${FOURTH_ROW_ID}:none`);
  await seekTo(page, 0.3);
  const from = (await engine(page)).a;
  await size(page, 8).click();
  const eight = await secondsOf(page, 8);
  await jumpForward(page).click();
  const to = await engineMovedFrom(page, from);
  expect(to - from).toBeCloseTo(eight, 1);
  await jumpBack(page).click();
  expect(await engineMovedFrom(page, to)).toBeCloseTo(from, 1);
});

test("a track with neither a grid nor a BPM does not jump", async ({ page }) => {
  await open(page);
  const bpms = page.locator('[role="gridcell"][data-col="bpm"]');
  let bare = -1;
  for (let i = 0; i < (await bpms.count()); i += 1) {
    if ((await bpms.nth(i).innerText()).trim() === "") { bare = i; break; }
  }
  expect(bare, "the mock list has an unanalysed row").toBeGreaterThanOrEqual(0);
  await titles(page).nth(bare).dblclick();
  await expect(playButton(page)).toBeEnabled();
  await seekTo(page, 0.3);
  await expect.poll(async () => (await engine(page)).a).toBeGreaterThan(1);
  const from = (await engine(page)).a;
  await expect(player(page).getByTestId("compact-player-tempo")).toHaveText("--");
  await jumpForward(page).click();
  await jumpBack(page).click();
  await page.waitForTimeout(400);
  expect((await engine(page)).a).toBeCloseTo(from, 3);
});

test("at the start a jump back runs into pre-roll; at the end a jump that would pass the end is not made", async ({ page }) => {
  await load(page);
  await size(page, 4).click();
  const four = await secondsOf(page, 4);
  await seekTo(page, 0);
  await expect.poll(async () => (await engine(page)).a).toBeLessThan(0.5);
  const start = (await engine(page)).a;
  await jumpBack(page).click();
  expect(await engineMovedFrom(page, start)).toBeCloseTo(start - four, 1);

  // Near the end: 4 beats fit, 32 do not.
  const total = await lengthSeconds(page);
  await seekTo(page, (total - 3 * four) / total);
  await expect.poll(async () => (await engine(page)).a).toBeGreaterThan(total - 4 * four);
  const near = (await engine(page)).a;
  await size(page, 32).click();
  await jumpForward(page).click();
  await page.waitForTimeout(400);
  expect((await engine(page)).a).toBeCloseTo(near, 3);
  await size(page, 4).click();
  await jumpForward(page).click();
  expect(await engineMovedFrom(page, near)).toBeCloseTo(near + four, 1);
});

test("while playing, a jump moves the running head by the size", async ({ page }) => {
  await load(page);
  await seekTo(page, 0.3);
  await size(page, 16).click();
  const sixteen = await secondsOf(page, 16);
  await playButton(page).click();
  await expect(pauseButton(page)).toBeVisible();
  await expect.poll(async () => (await engine(page)).aNow).toBeGreaterThan(0);
  // The head runs at the deck's own speed, so what the jump added is what
  // the head moved beyond the time that passed.
  const before = await engine(page);
  await jumpForward(page).click();
  await expect.poll(async () => {
    const now = await engine(page);
    return now.aNow - before.aNow - (now.at - before.at) / 1000;
  }, { timeout: 5000 }).toBeGreaterThan(sixteen - 0.2);
  const after = await engine(page);
  expect(after.aNow - before.aNow - (after.at - before.at) / 1000).toBeCloseTo(sixteen, 0);
  await expect(pauseButton(page)).toBeVisible();
});

test("with a loop running, a jump moves the loop and the head together", async ({ page }) => {
  await load(page);
  await seekTo(page, 0.4);
  await expect.poll(async () => (await engine(page)).a).toBeGreaterThan(1);
  // 6 is the Export preset's 4 Beat Loop; the arrows' owner does not matter
  // for it, but the player is clicked so the keys are the deck's.
  await player(page).getByTestId("compact-player-title").click();
  await page.keyboard.press("6");
  await expect.poll(async () => (await engine(page)).loopingA).toBe(true);
  const loop = await engine(page);
  await size(page, 8).click();
  const eight = await secondsOf(page, 8);
  await jumpForward(page).click();
  await expect.poll(async () => (await engine(page)).aLoopIn - loop.aLoopIn).toBeGreaterThan(0.1);
  const moved = await engine(page);
  expect(moved.aLoopIn - loop.aLoopIn).toBeCloseTo(eight, 1);
  expect(moved.aLoopOut - loop.aLoopOut).toBeCloseTo(eight, 1);
  expect(moved.a).toBeGreaterThanOrEqual(moved.aLoopIn - 0.01);
  expect(moved.a).toBeLessThan(moved.aLoopOut);
  expect(moved.loopingA).toBe(true);
});

for (const scale of [{ stop: 4, factor: 1 }, { stop: 6, factor: 1.25 }, { stop: 1, factor: 0.7 }]) {
  for (const width of [1280, 640, 436]) {
    test(`the overview's canvas is backed at the pixels it covers at ${width} px and ${scale.factor * 100}%`, async ({ page }) => {
      await page.setViewportSize({ width, height: 600 });
      await load(page, "", 3, { uiScale: scale.stop });
      await expect.poll(async () => overview(page).locator("canvas").evaluate((canvas: HTMLCanvasElement) => {
        // The browser build scales with CSS zoom on the root, which
        // `devicePixelRatio` does not include; the canvas must.
        const zoom = Number(document.documentElement.style.getPropertyValue("zoom") || 1);
        const ratio = window.devicePixelRatio * zoom;
        const host = canvas.parentElement;
        if (!host) return "no host";
        const want = { w: Math.round(host.clientWidth * ratio), h: Math.round(host.clientHeight * ratio) };
        return Math.abs(canvas.width - want.w) <= 1 && Math.abs(canvas.height - want.h) <= 1
          ? "ok" : `${canvas.width}x${canvas.height} for ${want.w}x${want.h}`;
      })).toBe("ok");
    });
  }
}
