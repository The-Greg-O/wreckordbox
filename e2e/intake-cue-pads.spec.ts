/**
 * Cue pads, grid nudge and tap tempo in Intake (wbx-64): the row under the
 * large player's detail waveform. Eight hot cue pads A to H show which cues
 * are set, in their colours and at their positions; an empty pad sets a hot
 * cue at the playhead, a set one jumps to it and plays, and its ✕ clears it.
 * The grid nudges earlier or later and takes a tapped tempo.
 *
 * Every edit is the deck's existing one and a step of the one shared Undo
 * history, so Ctrl+Z takes back the DJ's latest step whatever has the
 * focus. Nothing acts on a track until its own cues and grid are read;
 * Mark Clean, P and Skip wait for the track's cue and grid writes and stop
 * on a failed one, which stays with its track.
 *
 * The mock keeps cues and grids per track; `src/ipc/mock-cue-pads.ts` lists
 * the hooks used here (`__hotCuesOf`, `__gridOf`, `__gridBeatsOf`,
 * `__undoLabels`, the hold and fail switches and the write counts).
 */
import { expect, test, type Page } from "@playwright/test";
import { player, seedShell } from "./compactPlayer";

type HotCue = { letter: string; positionMs: number; colour: string | null };
type Grid = { beatMs: number; bpmX100: number; locked: boolean } | null;
type Hooks = Record<string, (...args: unknown[]) => unknown>;

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const queue = (page: Page) => page.getByRole("listbox", { name: "Intake queue" });
const options = (page: Page) => queue(page).getByRole("option");
const panel = (page: Page) => page.getByRole("region", { name: "Track" });
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
const failed = (page: Page) => player(page).getByTestId("intake-pads-failed");

const call = (page: Page, name: string, ...args: unknown[]) =>
  page.evaluate(([hook, rest]) => (window as unknown as Hooks)[hook]!(...(rest)), [name, args] as const);
const hotCuesOf = async (page: Page, id: string) => (await call(page, "__hotCuesOf", id)) as HotCue[];
const gridOf = async (page: Page, id: string) => (await call(page, "__gridOf", id)) as Grid;
const beatsOf = async (page: Page, id: string) => (await call(page, "__gridBeatsOf", id)) as number[][];
const undoLabels = async (page: Page) => (await call(page, "__undoLabels")) as string[];
const cueWrites = async (page: Page) => (await call(page, "__cueWrites")) as number;
const gridWrites = async (page: Page, kind?: string) => (await call(page, "__gridWrites", kind)) as number;
const deckSeconds = (page: Page) =>
  page.evaluate(() => (window as unknown as { __deckSeconds: () => { aNow: number } }).__deckSeconds().aNow);

/** `0:32.1`, as the pads print a position. */
function padTime(ms: number): string {
  const tenths = Math.floor(ms / 100);
  const minutes = Math.floor(tenths / 600);
  const seconds = Math.floor((tenths % 600) / 10);
  return `${minutes}:${String(seconds).padStart(2, "0")}.${tenths % 10}`;
}

/** `#3CEB50` as the computed style says it: `rgb(60, 235, 80)`. */
function rgb(hex: string): string {
  const n = Number.parseInt(hex.replace("#", ""), 16);
  return `rgb(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255})`;
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

/** The queue's first rows with a grid that `wanted` accepts by their cues. */
async function findTracks(page: Page, wanted: (cues: HotCue[]) => boolean, count = 1): Promise<{ id: string; at: number; cues: HotCue[] }[]> {
  const found: { id: string; at: number; cues: HotCue[] }[] = [];
  const total = await options(page).count();
  for (let at = 0; at < total && found.length < count; at += 1) {
    const id = (await options(page).nth(at).getAttribute("data-track"))!;
    const cues = await hotCuesOf(page, id);
    if (wanted(cues) && (await gridOf(page, id)) !== null) found.push({ id, at, cues });
  }
  if (found.length < count) throw new Error("not enough queued tracks fit");
  return found;
}

/** Chooses the queue's row `at` and waits until its cues and grid are read. */
async function choose(page: Page, at: number): Promise<void> {
  const row = options(page).nth(at);
  const title = await row.locator("span").first().innerText();
  await row.click();
  await expect(loadedTitle(page)).toHaveText(title);
  await expect(pads(page)).toHaveAttribute("data-ready");
  await expect(grid(page)).toHaveAttribute("data-ready");
}

async function chooseTrack(page: Page, wanted: (cues: HotCue[]) => boolean): Promise<{ id: string; at: number; cues: HotCue[] }> {
  const [track] = await findTracks(page, wanted);
  await choose(page, track!.at);
  return track!;
}

/** Moves the paused playhead on by a beat jump, so a new cue lands somewhere other than 0. */
async function moveOn(page: Page): Promise<number> {
  const before = await deckSeconds(page);
  await player(page).getByRole("button", { name: "Jump forward", exact: true }).click();
  await expect.poll(() => deckSeconds(page)).toBeGreaterThan(before + 1);
  return deckSeconds(page);
}

test("eight pads show which hot cues are set, each in its own colour and at its position", async ({ page }) => {
  await openIntake(page);
  // A set of coloured cues: each its own colour.
  const { cues } = await chooseTrack(page, (list) => list.length > 0 && list.length < 8
    && list.every((c) => c.colour !== null) && new Set(list.map((c) => c.colour)).size === list.length);
  await expect(pads(page).getByRole("button", { name: /^Hot cue [A-H]$/ })).toHaveCount(8);
  for (const letter of "ABCDEFGH") {
    const cue = cues.find((c) => c.letter === letter);
    if (cue) {
      await expect(pad(page, letter)).toHaveAttribute("data-set", "");
      await expect(pad(page, letter)).toContainText(padTime(cue.positionMs));
      await expect(pad(page, letter)).toHaveCSS("border-top-color", rgb(cue.colour!));
    } else {
      await expect(pad(page, letter)).not.toHaveAttribute("data-set", "");
      await expect(pad(page, letter)).toContainText("unset");
    }
  }
});

test("an empty pad sets a hot cue at the playhead, a set pad jumps to it and plays, and its ✕ clears it", async ({ page }) => {
  await openIntake(page);
  const { id } = await chooseTrack(page, (list) => !list.some((c) => c.letter === "E"));
  const at = await moveOn(page);

  await pad(page, "E").click();
  await expect.poll(async () => (await hotCuesOf(page, id)).find((c) => c.letter === "E")?.positionMs ?? -1).toBeGreaterThan(0);
  const cue = (await hotCuesOf(page, id)).find((c) => c.letter === "E")!;
  expect(Math.abs(cue.positionMs - at * 1000)).toBeLessThan(60);
  await expect(pad(page, "E")).toContainText(padTime(cue.positionMs));
  await expect(player(page).getByTestId("compact-player-overview").locator('[data-cue="E"]')).toHaveCount(1);
  // Setting a cue leaves the transport alone.
  await expect(playButton(page)).toBeVisible();

  // Move away, then the set pad calls its cue: from the cue, and playing.
  await moveOn(page);
  await moveOn(page);
  await pad(page, "E").click();
  await expect(pauseButton(page)).toBeVisible();
  const landed = await deckSeconds(page);
  expect(landed).toBeGreaterThanOrEqual(cue.positionMs / 1000 - 0.05);
  expect(landed).toBeLessThan(cue.positionMs / 1000 + 0.6);
  await expect.poll(() => deckSeconds(page)).toBeGreaterThan(landed + 0.2);
  await pauseButton(page).click();

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

test("a hot cue set or cleared is one step of the shared Undo history, with Redo", async ({ page }) => {
  await openIntake(page);
  const { id, cues } = await chooseTrack(page, (list) => !list.some((c) => c.letter === "H"));
  await moveOn(page);
  await pad(page, "H").click();
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length + 1);

  await page.keyboard.press("Control+z");
  await expect.poll(() => hotCuesOf(page, id)).toEqual(cues);
  await expect(pad(page, "H")).toContainText("unset");

  await page.keyboard.press("Control+Shift+z");
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length + 1);
  await expect(pad(page, "H")).toHaveAttribute("data-set", "");

  await clear(page, "H").click();
  await expect.poll(() => hotCuesOf(page, id)).toEqual(cues);
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await hotCuesOf(page, id)).some((c) => c.letter === "H")).toBe(true);
});

test("Ctrl+Z follows the order of the DJ's steps: Energy, then a nudge, then focus elsewhere", async ({ page }) => {
  await openIntake(page);
  const { id } = await chooseTrack(page, () => true);
  const before = await beatsOf(page, id);
  const rating = async () => (await call(page, "__ratingOf", id)) as number;
  const energyBefore = await rating();
  const energy = energyBefore === 3 ? 2 : 3;
  await panel(page).getByRole("radio", { name: `Energy ${energy}` }).click();
  await expect.poll(rating).toBe(energy);
  await later(page).click();
  await expect.poll(() => beatsOf(page, id)).not.toEqual(before);
  const nudged = await beatsOf(page, id);
  // Focus elsewhere: the queue, away from the player.
  await queue(page).focus();

  await page.keyboard.press("Control+z");
  await expect.poll(() => beatsOf(page, id)).toEqual(before);
  expect(await rating()).toBe(energy);
  await page.keyboard.press("Control+z");
  await expect.poll(rating).toBe(energyBefore);
  expect(await beatsOf(page, id)).toEqual(before);

  await page.keyboard.press("Control+Shift+z");
  await expect.poll(rating).toBe(energy);
  expect(await beatsOf(page, id)).toEqual(before);
  await page.keyboard.press("Control+Shift+z");
  await expect.poll(() => beatsOf(page, id)).toEqual(nudged);
});

test("a nudge then a cleared cue: Ctrl+Z brings the cue back first, then the grid", async ({ page }) => {
  await openIntake(page);
  const { id, cues } = await chooseTrack(page, (list) => list.length > 0);
  const before = await beatsOf(page, id);
  await earlier(page).click();
  await expect.poll(() => beatsOf(page, id)).not.toEqual(before);
  const nudged = await beatsOf(page, id);
  await clear(page, cues[0]!.letter).click();
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length - 1);

  // The player still has the keyboard: Undo is the latest step all the same.
  await page.keyboard.press("Control+z");
  await expect.poll(() => hotCuesOf(page, id)).toEqual(cues);
  expect(await beatsOf(page, id)).toEqual(nudged);
  await page.keyboard.press("Control+z");
  await expect.poll(() => beatsOf(page, id)).toEqual(before);
});

test("under Library Protection the empty pads and the grid refuse, say why, and nothing is written", async ({ page }) => {
  await openIntake(page, { protect: true });
  const { id, cues } = await chooseTrack(page, (list) => list.length > 0 && list.length < 8);
  const before = await beatsOf(page, id);
  const unset = [..."ABCDEFGH"].find((l) => !cues.some((c) => c.letter === l))!;
  await expect(pad(page, unset)).toBeDisabled();
  await expect(pad(page, unset)).toHaveAttribute("title", /read-only/);
  await expect(clear(page, cues[0]!.letter)).toHaveCount(0);
  for (const control of [later(page), earlier(page), tap(page)]) {
    await expect(control).toBeDisabled();
    await expect(control).toHaveAttribute("title", /read-only/);
  }
  await pad(page, unset).click({ force: true });
  await later(page).click({ force: true });
  await tap(page).click({ force: true });
  expect(await cueWrites(page)).toBe(0);
  expect(await gridWrites(page)).toBe(0);
  expect(await hotCuesOf(page, id)).toEqual(cues);
  expect(await beatsOf(page, id)).toEqual(before);
  // A set pad still calls its cue: calling writes nothing.
  await expect(pad(page, cues[0]!.letter)).toBeEnabled();
});

test("a cue write the Library refuses is shown with its track, and the pad stays empty", async ({ page }) => {
  await openIntake(page);
  const { id, cues } = await chooseTrack(page, (list) => !list.some((c) => c.letter === "G"));
  await call(page, "__failCueWrites", "rekordbox is running");
  await moveOn(page);
  await pad(page, "G").click();
  await expect(failed(page)).toContainText("rekordbox is running");
  expect(await hotCuesOf(page, id)).toEqual(cues);
  await expect(pad(page, "G")).toContainText("unset");

  // Once a write to the track lands again the note goes.
  await call(page, "__failCueWrites", null);
  await pad(page, "G").click();
  await expect(pad(page, "G")).toHaveAttribute("data-set", "");
  await expect(failed(page)).toHaveCount(0);
});

test("the grid nudges a millisecond earlier and later, and Ctrl+Z puts back the whole grid", async ({ page }) => {
  await openIntake(page);
  const { id } = await chooseTrack(page, () => true);
  const start = (await gridOf(page, id))!;
  const before = await beatsOf(page, id);
  await expect(bpmReadout(page)).toHaveText((start.bpmX100 / 100).toFixed(2));

  await later(page).click();
  await expect.poll(async () => (await gridOf(page, id))!.beatMs).toBe(start.beatMs + 1);
  await page.waitForTimeout(300);
  await earlier(page).click();
  await page.waitForTimeout(300);
  await earlier(page).click();
  await expect.poll(async () => (await gridOf(page, id))!.beatMs).toBe(start.beatMs - 1);

  const steps = (await undoLabels(page)).filter((l) => l.startsWith("Shift Beat Grid")).length;
  expect(steps).toBeGreaterThanOrEqual(1);
  for (let n = 0; n < steps; n += 1) await page.keyboard.press("Control+z");
  await expect.poll(() => beatsOf(page, id)).toEqual(before);
});

test("a tap run is applied to the grid as the taps go, counts each tap, and is one Undo back to the whole grid", async ({ page }) => {
  await openIntake(page);
  const { id } = await chooseTrack(page, () => true);
  const before = await beatsOf(page, id);
  const steps = (await undoLabels(page)).length;
  for (let n = 0; n < 5; n += 1) {
    await tap(page).click();
    if (n < 4) await page.waitForTimeout(400);
  }
  // The grid takes the tapped tempo (taps 400 ms apart: about 150 BPM),
  // and the readout shows the grid's tempo once the run is over.
  await expect(bpmReadout(page)).not.toHaveAttribute("data-tapping");
  const applied = (await gridOf(page, id))!.bpmX100;
  expect(applied).toBeGreaterThan(11_000);
  expect(applied).toBeLessThan(20_000);
  await expect(bpmReadout(page)).toHaveText((applied / 100).toFixed(2));
  // Five taps in one run: four tempo writes, one Undo step. (On a busy
  // machine a tap delayed past the run's end begins a second run, a second
  // step and one write fewer, and one off by more than the outlier gate is
  // not counted; the exact counts are in the keyboard and palette tests.)
  const runs = (await undoLabels(page)).length - steps;
  expect(runs).toBeGreaterThanOrEqual(1);
  expect(runs).toBeLessThanOrEqual(2);
  const writes = await gridWrites(page, "tap");
  expect(writes).toBeGreaterThanOrEqual(runs);
  expect(writes).toBeLessThanOrEqual(5 - runs);
  expect((await undoLabels(page)).slice(steps)).toEqual(Array(runs).fill("Tap Tempo"));

  await page.waitForTimeout(1600);
  for (let n = 0; n < runs; n += 1) await page.keyboard.press("Control+z");
  await expect.poll(() => beatsOf(page, id)).toEqual(before);
});

test("a tap over tempo changes asks first, once a run: No keeps every beat, Yes replaces them and is undoable", async ({ page }) => {
  // Find an Intake track with a grid, then reload with its grid doubling
  // tempo half way.
  await openIntake(page);
  const [track] = await findTracks(page, () => true);
  await page.goto(`/?writable=1&grids=${track!.id}:double`);
  await tab(page, "Intake").click();
  await choose(page, track!.at);
  const id = track!.id;
  const before = await beatsOf(page, id);
  expect(new Set(before.map((b) => b[2])).size).toBe(2);
  const asked = async () => ((await page.evaluate(() => (window as unknown as { __confirmed?: string[] }).__confirmed ?? [])))
    .filter((q) => q.includes("tempo changes")).length;

  await page.evaluate(() => { (window as unknown as { __confirmAnswers: boolean[] }).__confirmAnswers = [false]; });
  await tap(page).click();
  await expect.poll(asked).toBe(1);
  await page.waitForTimeout(500);
  // No: nothing is written, every beat stays.
  expect(await gridWrites(page, "tap")).toBe(0);
  expect(await beatsOf(page, id)).toEqual(before);

  await page.evaluate(() => { (window as unknown as { __confirmAnswers: boolean[] }).__confirmAnswers = [true]; });
  await tap(page).click();
  await expect.poll(asked).toBe(2);
  await page.waitForTimeout(300);
  for (let n = 0; n < 4; n += 1) {
    await tap(page).click();
    if (n < 3) await page.waitForTimeout(400);
  }
  // Asked once for the run, however many taps it took.
  expect(await asked()).toBe(2);
  await expect.poll(async () => new Set((await beatsOf(page, id)).map((b) => b[2])).size).toBe(1);
  await page.waitForTimeout(1600);
  const runs = (await undoLabels(page)).filter((l) => l === "Tap Tempo").length;
  expect(runs).toBeGreaterThanOrEqual(1);
  for (let n = 0; n < runs; n += 1) await page.keyboard.press("Control+z");
  await expect.poll(() => beatsOf(page, id)).toEqual(before);
});

test("pads act only on the loaded track's own cues: a slow read after Mark Clean leaves them off", async ({ page }) => {
  await openIntake(page);
  const [first] = await findTracks(page, (list) => list.length > 0);
  await choose(page, first!.at);
  const firstCues = await hotCuesOf(page, first!.id);
  const firstTitle = await loadedTitle(page).innerText();
  // The next track's cue read is held: the previous track's cues must not
  // stand in for it.
  await call(page, "__holdCueReads", null);
  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
  await expect(loadedTitle(page)).not.toHaveText(firstTitle);
  await expect(pads(page)).not.toHaveAttribute("data-ready");
  for (const letter of "ABCDEFGH") {
    await expect(pad(page, letter)).toBeDisabled();
    await expect(pad(page, letter)).toContainText("…");
  }
  await expect(pads(page).getByRole("button", { name: /^Clear hot cue/ })).toHaveCount(0);
  await expect(player(page).getByTestId("compact-player-overview").locator("[data-cue]")).toHaveCount(0);
  await pad(page, firstCues[0]!.letter).click({ force: true });
  expect(await cueWrites(page)).toBe(0);
  expect(await hotCuesOf(page, first!.id)).toEqual(firstCues);

  await call(page, "__releaseCueReads");
  await expect(pads(page)).toHaveAttribute("data-ready");
  const loaded = (await queue(page).locator('[role="option"][aria-selected="true"]').getAttribute("data-track"))!;
  expect(loaded).not.toBe(first!.id);
  const shownCues = await hotCuesOf(page, loaded);
  for (const letter of "ABCDEFGH") {
    const cue = shownCues.find((c) => c.letter === letter);
    await expect(pad(page, letter)).toContainText(cue ? padTime(cue.positionMs) : "unset");
  }
});

test("Skip waits for the track's grid writes on their way, and they land on that track", async ({ page }) => {
  await openIntake(page);
  const [track] = await findTracks(page, () => true);
  await choose(page, track!.at);
  const start = (await gridOf(page, track!.id))!;
  const title = await loadedTitle(page).innerText();
  await call(page, "__holdGridWrites");
  await later(page).click();
  await later(page).click();
  await page.keyboard.press("]");
  await page.waitForTimeout(500);
  await expect(loadedTitle(page)).toHaveText(title);
  await call(page, "__releaseGridWrites");
  await expect(loadedTitle(page)).not.toHaveText(title);
  // Both presses, saved to the track they were made on.
  await expect.poll(async () => (await gridOf(page, track!.id))!.beatMs).toBe(start.beatMs + 2);
});

test("a refused grid write stops Mark Clean and stays with its track after moving on", async ({ page }) => {
  await openIntake(page);
  const [track] = await findTracks(page, () => true);
  await choose(page, track!.at);
  const before = await beatsOf(page, track!.id);
  const title = await loadedTitle(page).innerText();
  await call(page, "__failGridWrites", "The beat grid is locked.");
  await later(page).click();
  await panel(page).getByRole("button", { name: "Mark Clean and next" }).click();
  await expect(panel(page).getByText(/Not saved: grid: The beat grid is locked\./)).toBeVisible();
  await expect(loadedTitle(page)).toHaveText(title);
  expect(await call(page, "__prepColourOf", track!.id)).toBe(0);
  expect(await beatsOf(page, track!.id)).toEqual(before);
  await expect(failed(page)).toContainText("The beat grid is locked.");

  // Choosing another track: the failure is listed with the track it came from.
  await call(page, "__failGridWrites", null);
  await choose(page, track!.at + 1);
  const listed = page.getByTestId("intake-deck-failures");
  await expect(listed.locator(`[data-track="${track!.id}"]`)).toContainText("The beat grid is locked.");
  await expect(failed(page)).toHaveCount(0);
  await listed.getByRole("button", { name: "Dismiss" }).click();
  await expect(listed).toHaveCount(0);
});

test("pad presses are queued with their track and playhead: a slow write loses none", async ({ page }) => {
  await openIntake(page);
  const { id, cues } = await chooseTrack(page, (list) => !list.some((c) => "EF".includes(c.letter)));
  await call(page, "__holdCueWrites");
  const atE = await moveOn(page);
  await pad(page, "E").click();
  await expect(pad(page, "E")).toHaveAttribute("data-pending", "set");
  const atF = await moveOn(page);
  await pad(page, "F").click();
  // A second press on a slot on its way does nothing.
  await pad(page, "E").click({ force: true });
  await call(page, "__releaseCueWrites");
  await expect.poll(async () => (await hotCuesOf(page, id)).length).toBe(cues.length + 2);
  const now = await hotCuesOf(page, id);
  expect(Math.abs(now.find((c) => c.letter === "E")!.positionMs - atE * 1000)).toBeLessThan(60);
  expect(Math.abs(now.find((c) => c.letter === "F")!.positionMs - atF * 1000)).toBeLessThan(60);
  expect(await cueWrites(page)).toBe(2);
  // Neither call started the deck.
  await expect(playButton(page)).toBeVisible();
});

test("a slot written but not yet read back takes no second cue", async ({ page }) => {
  await openIntake(page);
  const { id, cues } = await chooseTrack(page, (list) => !list.some((c) => c.letter === "G"));
  await moveOn(page);
  await call(page, "__holdCueReads", id);
  await pad(page, "G").click();
  await expect.poll(() => cueWrites(page)).toBe(1);
  // Written, not yet read back: still on its way.
  await expect(pad(page, "G")).toHaveAttribute("data-pending", "set");
  await pad(page, "G").click({ force: true });
  await page.waitForTimeout(300);
  expect(await cueWrites(page)).toBe(1);
  await call(page, "__releaseCueReads");
  await expect(pad(page, "G")).toHaveAttribute("data-set", "");
  await expect(pad(page, "G")).not.toHaveAttribute("data-pending");
  expect((await hotCuesOf(page, id)).filter((c) => c.letter === "G")).toHaveLength(1);
  expect((await hotCuesOf(page, id)).length).toBe(cues.length + 1);
});

test("keys: a held Enter sets a pad once, a held nudge is one step, a held Space taps once, Space never plays", async ({ page }) => {
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

  // A nudge is meant to repeat: a key and its repeats, a millisecond each,
  // all one hold and one Undo step, in at most a few writes.
  const start = (await gridOf(page, id))!;
  const steps = (await undoLabels(page)).length;
  const writes = await gridWrites(page, "nudge");
  await later(page).focus();
  for (let n = 0; n < 6; n += 1) await page.keyboard.down("Enter");
  await page.keyboard.up("Enter");
  await expect.poll(async () => (await gridOf(page, id))!.beatMs).toBe(start.beatMs + 6);
  await page.waitForTimeout(400);
  expect((await undoLabels(page)).length).toBe(steps + 1);
  expect(await gridWrites(page, "nudge") - writes).toBeLessThanOrEqual(3);
  await expect(playButton(page)).toBeVisible();
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await gridOf(page, id))!.beatMs).toBe(start.beatMs);

  // A held Space on Tap taps once: two held presses are two taps, one write.
  const taps = await gridWrites(page, "tap");
  await tap(page).focus();
  for (let press = 0; press < 2; press += 1) {
    await page.keyboard.down(" ");
    await page.keyboard.down(" ");
    await page.keyboard.down(" ");
    await page.keyboard.up(" ");
    await page.waitForTimeout(450);
  }
  await expect.poll(() => gridWrites(page, "tap")).toBe(taps + 1);
  await page.waitForTimeout(300);
  expect(await gridWrites(page, "tap")).toBe(taps + 1);
  await expect(playButton(page)).toBeVisible();

  // The view keys still switch views from a pad.
  await pad(page, "A").focus();
  await page.keyboard.press("1");
  await expect(tab(page, "Library")).toHaveAttribute("aria-selected", "true");
});

test("the palette has Tap Tempo, greyed exactly when the button is", async ({ page }) => {
  await openIntake(page);
  await chooseTrack(page, () => true);
  const palette = page.getByRole("dialog", { name: "Command palette" });
  const entry = palette.getByRole("listbox", { name: "Actions" }).getByRole("option", { name: /^Tap Tempo/ }).first();
  const runTap = async () => {
    await page.keyboard.press("Control+Shift+P");
    await palette.getByRole("combobox", { name: "Search actions" }).fill("Tap Tempo");
    await expect(entry).not.toHaveAttribute("aria-disabled", "true");
    await page.keyboard.press("Enter");
    await expect(palette).toHaveCount(0);
  };
  const before = await gridWrites(page, "tap");
  // Two taps from the palette are a run: the second writes the tempo.
  // Alone, one tap writes nothing.
  await runTap();
  await page.waitForTimeout(300);
  expect(await gridWrites(page, "tap")).toBe(before);
  await runTap();
  await expect.poll(() => gridWrites(page, "tap")).toBe(before + 1);
});

test("the palette's Tap Tempo is greyed under Library Protection, as the button is", async ({ page }) => {
  await openIntake(page, { protect: true });
  await chooseTrack(page, () => true);
  await expect(tap(page)).toBeDisabled();
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await page.keyboard.press("Control+Shift+P");
  await palette.getByRole("combobox", { name: "Search actions" }).fill("Tap Tempo");
  await expect(palette.getByRole("listbox", { name: "Actions" }).getByRole("option", { name: /^Tap Tempo/ }).first())
    .toHaveAttribute("aria-disabled", "true");
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
