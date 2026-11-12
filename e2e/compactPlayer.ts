/**
 * Shared locators and readers for the compact player's specs (ticket #51):
 * `compact-player.spec.ts` (what it shows, its keys and its sizes) and
 * `compact-player-jumps.spec.ts` (where a jump lands, and its canvases).
 */
import { expect, type Page } from "@playwright/test";

/**
 * The track id of the list's fourth row, the one `load` loads by default,
 * for the mock's `?grids=<id>:double|none` grid fixtures. The list is not in
 * id order, so the row's position is not its id.
 */
export const FOURTH_ROW_ID = "101325";

export const player = (page: Page) => page.getByTestId("shell-player").getByRole("region", { name: "Preview player" });
export const titles = (page: Page) => page.locator('[role="gridcell"][data-col="title"]');
export const overview = (page: Page) => player(page).getByTestId("compact-player-overview");
export const sizes = (page: Page) => player(page).getByRole("radiogroup", { name: "Beat jump size" });
export const size = (page: Page, beats: number) => sizes(page).getByRole("radio", { name: `${beats} beats`, exact: true });
export const jumpBack = (page: Page) => player(page).getByRole("button", { name: "Jump back", exact: true });
export const jumpForward = (page: Page) => player(page).getByRole("button", { name: "Jump forward", exact: true });
export const playButton = (page: Page) => player(page).getByRole("button", { name: "Play", exact: true });
export const pauseButton = (page: Page) => player(page).getByRole("button", { name: "Pause", exact: true });

/**
 * Turns the shell on with the given extra view preferences, once: a later
 * page or reload in the same context reads what the app stored since.
 */
export async function seedShell(page: Page, view: Record<string, unknown> = {}): Promise<void> {
  await page.addInitScript((extra) => {
    if (sessionStorage.getItem("compact-player-seeded")) return;
    sessionStorage.setItem("compact-player-seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify({
      advanced: { protectLibrary: false },
      view: { viewShell: true, ...extra },
    }));
  }, view);
}

export async function open(page: Page, query = "", view: Record<string, unknown> = {}): Promise<void> {
  await seedShell(page, view);
  await page.goto(`/${query}`);
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
}

/** Loads a row (the fourth by default: analysed, with a tempo, a key, hot cues and phrases). */
export async function load(page: Page, query = "", row = 3, view: Record<string, unknown> = {}): Promise<void> {
  await open(page, query, view);
  await titles(page).nth(row).dblclick();
  await expect(playButton(page)).toBeEnabled();
}

/** The position readout's elapsed part in seconds, signed: `-0:01.9 / 5:48` is -1.9. */
export async function elapsedSeconds(page: Page): Promise<number> {
  const text = (await player(page).getByTestId("compact-player-elapsed").textContent()) ?? "";
  const [, sign, minutes, seconds] = /(-?)(\d+):(\d+(?:\.\d)?)/.exec(text) ?? [];
  return (sign ? -1 : 1) * (Number(minutes) * 60 + Number(seconds));
}

/** The tempo readout as a number. */
export async function tempo(page: Page): Promise<number> {
  const readout = player(page).getByTestId("compact-player-tempo");
  await expect(readout).toHaveText(/^[1-9]\d*\.\d\d$/);
  return Number(await readout.textContent());
}

/** How long `beats` last at the tempo the readout shows. */
export async function secondsOf(page: Page, beats: number): Promise<number> {
  return (beats * 60) / (await tempo(page));
}

/** The readout shows tenths, so a landing is right to within one of them. */
export async function expectHeadAt(page: Page, seconds: number): Promise<void> {
  await expect.poll(() => elapsedSeconds(page)).toBeCloseTo(seconds, 0);
  expect(Math.abs((await elapsedSeconds(page)) - seconds)).toBeLessThanOrEqual(0.11);
}

export interface Engine {
  /** Deck A's head at the last tick, in seconds (negative in pre-roll). */
  a: number;
  /** Deck A's head brought up to now. */
  aNow: number;
  aLoopIn: number;
  aLoopOut: number;
  loopingA: boolean;
  /** `performance.now()` when it was read. */
  at: number;
}

/** Where the mock engine's deck A is: the engine's own position, not the readout. */
export async function engine(page: Page): Promise<Engine> {
  return page.evaluate(() => {
    const read = (window as unknown as { __deckSeconds: () => Omit<Engine, "at"> }).__deckSeconds();
    return { a: read.a, aNow: read.aNow, aLoopIn: read.aLoopIn, aLoopOut: read.aLoopOut, loopingA: read.loopingA, at: performance.now() };
  });
}

/** Waits for the engine head to settle somewhere other than `from`, and returns it. */
export async function engineMovedFrom(page: Page, from: number): Promise<number> {
  await expect.poll(async () => Math.abs((await engine(page)).a - from)).toBeGreaterThan(0.05);
  return (await engine(page)).a;
}

/** Moves the head by a click on the overview, `fraction` of the way along. */
export async function seekTo(page: Page, fraction: number): Promise<void> {
  const box = await overview(page).boundingBox();
  if (!box) throw new Error("no overview");
  await page.mouse.click(box.x + Math.max(1, box.width * fraction), box.y + box.height / 2);
}
