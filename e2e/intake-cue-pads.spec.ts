/**
 * Cue pads, grid nudge and tap tempo in Intake (wbx-64): the row under the
 * large player's detail waveform. Eight hot cue pads A to H show which cues
 * are set, in their colours and at their positions; an empty pad sets a hot
 * cue at the playhead, a set one jumps to it, and its ✕ clears it. The grid
 * nudges a millisecond earlier or later and takes a tapped tempo.
 *
 * Every edit is the deck's existing one: cues through the backend's cue
 * commands (one entry each of the shared Undo history), the grid through
 * the grid editor (its own Undo, which Ctrl+Z reaches while the player has
 * the keyboard). The mock keeps both per track; `__hotCuesOf(id)` and
 * `__gridOf(id)` read what it holds (`src/ipc/mock-cue-pads.ts`).
 */
import { expect, test, type Page } from "@playwright/test";
import { player, seedShell } from "./compactPlayer";

type HotCue = { letter: string; positionMs: number };
type Grid = { beatMs: number; bpmX100: number; locked: boolean } | null;

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const queue = (page: Page) => page.getByRole("listbox", { name: "Intake queue" });
const options = (page: Page) => queue(page).getByRole("option");
const pads = (page: Page) => player(page).getByRole("group", { name: "Hot cue pads" });
const pad = (page: Page, letter: string) => pads(page).getByRole("button", { name: `Hot cue ${letter}`, exact: true });
const clear = (page: Page, letter: string) => pads(page).getByRole("button", { name: `Clear hot cue ${letter}`, exact: true });
const grid = (page: Page) => player(page).getByRole("group", { name: "Beat grid" });
const earlier = (page: Page) => grid(page).getByRole("button", { name: "Shift the grid earlier", exact: true });
const later = (page: Page) => grid(page).getByRole("button", { name: "Shift the grid later", exact: true });
const tap = (page: Page) => grid(page).getByRole("button", { name: "Tap the tempo", exact: true });
const bpmReadout = (page: Page) => grid(page).getByTestId("intake-grid-bpm");
const loadedTitle = (page: Page) => player(page).getByTestId("compact-player-title");
const playButton = (page: Page) => player(page).getByRole("button", { name: "Play", exact: true });
const pauseButton = (page: Page) => player(page).getByRole("button", { name: "Pause", exact: true });

const hotCuesOf = (page: Page, id: string) =>
  page.evaluate((track) => (window as unknown as { __hotCuesOf: (id: string) => HotCue[] }).__hotCuesOf(track), id);
const gridOf = (page: Page, id: string) =>
  page.evaluate((track) => (window as unknown as { __gridOf: (id: string) => Grid }).__gridOf(track), id);
const cueWrites = (page: Page) => page.evaluate(() => (window as unknown as { __cueWrites: () => number }).__cueWrites());
const deckSeconds = (page: Page) =>
  page.evaluate(() => (window as unknown as { __deckSeconds: () => { a: number; aNow: number } }).__deckSeconds().aNow);

/** `0:32.1`, as the pads print a position. */
function padTime(ms: number): string {
  const tenths = Math.floor(ms / 100);
  const minutes = Math.floor(tenths / 600);
  const seconds = Math.floor((tenths % 600) / 10);
  return `${minutes}:${String(seconds).padStart(2, "0")}.${tenths % 10}`;
}

async function openIntake(page: Page, { protect = false, query = "?writable=1" } = {}): Promise<void> {
  if (protect) {
    await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
      advanced: { protectLibrary: true }, view: { viewShell: true },
    })));
  } else {
    await seedShell(page);
  }
  await page.goto(`/${query}`);
  await expect(page.getByTestId("shell")).toBeVisible();
  await tab(page, "Intake").click();
  await expect(options(page).first()).toBeVisible();
}

/**
 * Chooses the first queued track with a grid that `wanted` accepts, by its
 * cues, and waits for the player to hold it.
 */
async function chooseTrack(page: Page, wanted: (cues: HotCue[]) => boolean): Promise<{ id: string; cues: HotCue[] }> {
  const count = await options(page).count();
  for (let at = 0; at < count; at += 1) {
    const row = options(page).nth(at);
    const id = (await row.getAttribute("data-track"))!;
    const cues = await hotCuesOf(page, id);
    if (!wanted(cues) || (await gridOf(page, id)) === null) continue;
    const title = await row.locator("span").first().innerText();
    await row.click();
    await expect(loadedTitle(page)).toHaveText(title);
    await expect(pads(page)).toBeVisible();
    return { id, cues };
  }
  throw new Error("no queued track fits");
}

/** Moves the paused playhead on by a beat jump, so a new cue lands somewhere other than 0. */
async function moveOn(page: Page): Promise<number> {
  const before = await deckSeconds(page);
  await player(page).getByRole("button", { name: "Jump forward", exact: true }).click();
  await expect.poll(() => deckSeconds(page)).toBeGreaterThan(before + 1);
  return deckSeconds(page);
}

test("eight pads show which hot cues are set, in their colours and at their positions", async ({ page }) => {
  await openIntake(page);
  const { cues } = await chooseTrack(page, (list) => list.length > 0 && list.length < 8);
  await expect(pads(page).getByRole("button", { name: /^Hot cue [A-H]$/ })).toHaveCount(8);
  for (const letter of "ABCDEFGH") {
    const cue = cues.find((c) => c.letter === letter);
    if (cue) {
      await expect(pad(page, letter)).toHaveAttribute("data-set", "");
      await expect(pad(page, letter)).toContainText(padTime(cue.positionMs));
    } else {
      await expect(pad(page, letter)).not.toHaveAttribute("data-set", "");
      await expect(pad(page, letter)).toContainText("unset");
    }
  }
  const set = cues[0]!.letter;
  const unset = [..."ABCDEFGH"].find((l) => !cues.some((c) => c.letter === l))!;
  const colour = (letter: string) => pad(page, letter).evaluate((el) => getComputedStyle(el).borderTopColor);
  expect(await colour(set)).not.toBe(await colour(unset));
});

test("an empty pad sets a hot cue at the playhead, a set pad jumps to it, and its ✕ clears it", async ({ page }) => {
  await openIntake(page);
  const { id } = await chooseTrack(page, (list) => !list.some((c) => c.letter === "E"));
  const at = await moveOn(page);

  await pad(page, "E").click();
  await expect.poll(async () => (await hotCuesOf(page, id)).find((c) => c.letter === "E")?.positionMs ?? -1)
    .toBeGreaterThan(0);
  const cue = (await hotCuesOf(page, id)).find((c) => c.letter === "E")!;
  expect(Math.abs(cue.positionMs - at * 1000)).toBeLessThan(60);
  await expect(pad(page, "E")).toContainText(padTime(cue.positionMs));
  await expect(player(page).getByTestId("compact-player-overview").locator('[data-cue="E"]')).toHaveCount(1);
  await expect(playButton(page)).toBeVisible();

  // Move away, then the set pad calls its cue: the head goes back to it.
  await moveOn(page);
  await moveOn(page);
  await pad(page, "E").click();
  await expect.poll(async () => Math.abs((await deckSeconds(page)) - cue.positionMs / 1000)).toBeLessThan(0.6);
  if (await pauseButton(page).isVisible()) await pauseButton(page).click();

  // The ✕ is a button of its own, operable by keyboard.
  await clear(page, "E").focus();
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await hotCuesOf(page, id)).some((c) => c.letter === "E")).toBe(false);
  await expect(pad(page, "E")).toContainText("unset");
  await expect(clear(page, "E")).toHaveCount(0);
});

test("clearing the last hot cue and setting one again change what the track still needs", async ({ page }) => {
  await openIntake(page);
  const { id, cues } = await chooseTrack(page, (list) => list.length > 0);
  const row = queue(page).locator(`[data-track="${id}"]`);
  await expect(row.getByTestId("intake-needs")).not.toContainText("cues");
  for (const cue of cues) {
    await clear(page, cue.letter).click();
    await expect(pad(page, cue.letter)).toContainText("unset");
  }
  expect(await hotCuesOf(page, id)).toEqual([]);
  await expect(row.getByTestId("intake-needs")).toContainText("cues");
  await moveOn(page);
  await pad(page, "A").click();
  await expect(row.getByTestId("intake-needs")).not.toContainText("cues");
});

test("a hot cue set or cleared is one step of the shared Undo history", async ({ page }) => {
  await openIntake(page);
  const { id, cues } = await chooseTrack(page, (list) => !list.some((c) => c.letter === "H"));
  await moveOn(page);
  await pad(page, "H").click();
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length + 1);

  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length);
  await expect(pad(page, "H")).toContainText("unset");

  await page.keyboard.press("Control+Shift+z");
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length + 1);
  await expect(pad(page, "H")).toHaveAttribute("data-set", "");

  await clear(page, "H").click();
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length);
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await hotCuesOf(page, id)).some((c) => c.letter === "H")).toBe(true);
});

test("under Library Protection the empty pads and the grid refuse, say why, and nothing is written", async ({ page }) => {
  await openIntake(page, { protect: true });
  const { id, cues } = await chooseTrack(page, (list) => list.length > 0 && list.length < 8);
  const before = await gridOf(page, id);
  const unset = [..."ABCDEFGH"].find((l) => !cues.some((c) => c.letter === l))!;
  await expect(pad(page, unset)).toBeDisabled();
  await expect(pad(page, unset)).toHaveAttribute("title", /read-only/);
  await expect(clear(page, cues[0]!.letter)).toHaveCount(0);
  await expect(later(page)).toBeDisabled();
  await expect(earlier(page)).toBeDisabled();
  await expect(tap(page)).toBeDisabled();
  await pad(page, unset).click({ force: true });
  await later(page).click({ force: true });
  expect(await cueWrites(page)).toBe(0);
  expect(await hotCuesOf(page, id)).toEqual(cues);
  expect(await gridOf(page, id)).toEqual(before);
  // A set pad still calls its cue: calling writes nothing.
  await expect(pad(page, cues[0]!.letter)).toBeEnabled();
});

test("a cue write the Library refuses is shown, and the pad stays empty", async ({ page }) => {
  await openIntake(page);
  const { id, cues } = await chooseTrack(page, (list) => !list.some((c) => c.letter === "G"));
  await page.evaluate(() => (window as unknown as { __failCueWrites: (m: string | null) => void })
    .__failCueWrites("rekordbox is running"));
  await moveOn(page);
  await pad(page, "G").click();
  await expect(player(page).getByTestId("intake-pads-failed")).toContainText("rekordbox is running");
  expect(await hotCuesOf(page, id)).toEqual(cues);
  await expect(pad(page, "G")).toContainText("unset");

  // Once a write lands again the note goes.
  await page.evaluate(() => (window as unknown as { __failCueWrites: (m: string | null) => void }).__failCueWrites(null));
  await pad(page, "G").click();
  await expect(pad(page, "G")).toHaveAttribute("data-set", "");
  await expect(player(page).getByTestId("intake-pads-failed")).toHaveCount(0);
});

test("the grid nudges a millisecond earlier and later, and Ctrl+Z takes a nudge back", async ({ page }) => {
  await openIntake(page);
  const { id } = await chooseTrack(page, () => true);
  const start = (await gridOf(page, id))!;
  await expect(bpmReadout(page)).toHaveText((start.bpmX100 / 100).toFixed(2));

  await later(page).click();
  await expect.poll(async () => (await gridOf(page, id))!.beatMs).toBe(start.beatMs + 1);
  await earlier(page).click();
  await earlier(page).click();
  await expect.poll(async () => (await gridOf(page, id))!.beatMs).toBe(start.beatMs - 1);

  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await gridOf(page, id))!.beatMs).toBe(start.beatMs);
});

test("a tapped tempo is applied to the grid and is one step to undo", async ({ page }) => {
  await openIntake(page);
  const { id } = await chooseTrack(page, () => true);
  const start = (await gridOf(page, id))!;
  // The moments the taps went down, as the page saw them.
  await tap(page).evaluate((el) => {
    const moments: number[] = [];
    (window as unknown as { __taps: number[] }).__taps = moments;
    el.addEventListener("pointerdown", () => moments.push(performance.now()), true);
  });
  for (let n = 0; n < 5; n += 1) {
    await tap(page).click();
    if (n < 4) await page.waitForTimeout(500);
  }
  const taps = await page.evaluate(() => (window as unknown as { __taps: number[] }).__taps);
  expect(taps).toHaveLength(5);
  // While tapping the readout shows the tempo the taps describe; that is
  // the tempo the grid takes. (A tap late past the run's timeout on a busy
  // machine starts a new run, so the page's own timing is only a bound.)
  await expect(bpmReadout(page)).toHaveAttribute("data-tapping");
  const shown = Number.parseFloat(await bpmReadout(page).innerText());
  await expect.poll(async () => (await gridOf(page, id))!.bpmX100).toBe(Math.round(shown * 100));
  const timed = (60_000 * 4) / (taps[4]! - taps[0]!);
  expect(Math.abs(shown - timed)).toBeLessThan(15);
  expect(Math.abs((await gridOf(page, id))!.bpmX100 - start.bpmX100)).toBeGreaterThan(0);
  const tapped = (await gridOf(page, id))!.bpmX100;
  await expect(bpmReadout(page)).toHaveText((tapped / 100).toFixed(2));

  // After the taps have timed out, one Undo puts the old tempo back.
  await page.waitForTimeout(2500);
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await gridOf(page, id))!.bpmX100).toBe(start.bpmX100);
});

test("keys: a held Enter sets a pad once, a held nudge repeats, and Space never reaches the deck", async ({ page }) => {
  await openIntake(page);
  const { id, cues } = await chooseTrack(page, (list) => !list.some((c) => c.letter === "F"));
  await moveOn(page);

  // Held: the repeats neither set again nor call the cue just set.
  await pad(page, "F").focus();
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length + 1);
  await page.waitForTimeout(300);
  expect(await cueWrites(page)).toBe(1);
  await expect(playButton(page)).toBeVisible();

  // Space on the ✕ clears; the deck stays paused.
  await clear(page, "F").focus();
  await page.keyboard.press(" ");
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length);
  await expect(playButton(page)).toBeVisible();

  // A nudge is meant to repeat: three key downs, three milliseconds.
  const start = (await gridOf(page, id))!;
  await later(page).focus();
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await expect.poll(async () => (await gridOf(page, id))!.beatMs).toBe(start.beatMs + 3);
  await expect(playButton(page)).toBeVisible();

  // A held Space on Tap taps once, not on every repeat; nothing plays.
  await tap(page).focus();
  await page.keyboard.down(" ");
  await page.keyboard.down(" ");
  await page.keyboard.up(" ");
  await expect(playButton(page)).toBeVisible();
  // The view keys still switch views from a pad.
  await pad(page, "A").focus();
  await page.keyboard.press("1");
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
});

test("640px: the pads and grid fit beside each other without spilling", async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 800 });
  await openIntake(page);
  await chooseTrack(page, () => true);
  const row = player(page).getByTestId("intake-cue-row");
  const box = (await row.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(640);
  for (const letter of "ABCDEFGH") {
    const b = (await pad(page, letter).boundingBox())!;
    expect(b.x + b.width).toBeLessThanOrEqual(640);
  }
  const t = (await tap(page).boundingBox())!;
  expect(t.x + t.width).toBeLessThanOrEqual(640);
});
