/**
 * The Stick view (wbx-71): the mounted Stick, Gig or Spare, protected or
 * unlocked, its last load and its space before and after; the playlists a
 * load includes; what the load will do by kind of change, with every change
 * one press away; too little space said and the load disabled; no Stick
 * said. Showing the view, choosing and the dry run write nothing; the load
 * goes through the window's Stick write gate (plan, Stick Protection, hold).
 * Since wbx-72 the hold is the view's own big button (`stick-hold-to-load.spec.ts`):
 * a protected Stick is unlocked in the view first, and there is no dialog.
 *
 * Mock hooks: `src/ipc/mock-stick-load.ts` and `mock-stick-safety.ts`.
 */
import { expect, test, type Page } from "@playwright/test";

import { open, pauseButton, player, titles } from "./compactPlayer";

const tab = (page: Page, name: string) => page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name });
const view = (page: Page) => page.getByTestId("stick-view");
const choose = (page: Page) => view(page).getByTestId("stick-choose");
const loadButton = (page: Page) => view(page).getByTestId("stick-load");

const DJ = "/Volumes/DJ STICK";
const TEST = "/Volumes/TEST";

async function call<T>(page: Page, name: string, ...args: unknown[]): Promise<T> {
  return page.evaluate(([hook, rest]) => {
    const hooks = window as unknown as Record<string, (...a: unknown[]) => T>;
    return hooks[hook]!(...rest);
  }, [name, args] as [string, unknown[]]);
}

const writes = (page: Page) => call<{ method: string; path: string }[]>(page, "__stickWrites");
const dryRuns = async (page: Page) => (await call<{ dryRuns: number }>(page, "__stickLoadReads")).dryRuns;

async function openStick(page: Page): Promise<void> {
  await open(page);
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-name")).toBeVisible();
}

/** The playlists offered (not folders), in tree order. */
const playlistRows = (page: Page) => choose(page).locator("label").filter({ hasNotText: "(folder" });

/** Ticks the first playlist offered; answers its name. */
async function chooseFirst(page: Page): Promise<string> {
  const row = playlistRows(page).first();
  const label = (await row.innerText()).trim();
  await row.getByRole("checkbox").check();
  return label;
}

test("the view shows the Stick's name, kind, protection, last load and space; choosing and the dry run write nothing", async ({ page }) => {
  await openStick(page);

  await expect(view(page).getByTestId("stick-name")).toHaveText("DJ STICK");
  await expect(view(page).getByTestId("stick-kind")).toHaveText("Gig stick");
  await expect(view(page).getByTestId("stick-protection")).toHaveText("Protected");
  await expect(view(page).getByTestId("stick-meta")).toContainText("Never loaded by wreckordbox");
  await expect(view(page).getByTestId("stick-space-line")).toHaveText("Free now 25.8 GB of 34.4 GB");
  // Nothing chosen yet: the view asks for playlists and the load waits.
  await expect(view(page).getByTestId("stick-summary-none")).toBeVisible();
  await expect(loadButton(page)).toBeDisabled();
  await expect(view(page).getByTestId("stick-load-why")).toHaveText("Choose at least one playlist to load.");

  const name = await chooseFirst(page);
  const summary = view(page).getByTestId("stick-summary");
  await expect(summary.locator('[data-change="added"]')).toContainText(`tracks added, across ${name}`);
  await expect(summary.locator('[data-change="playlists"]')).toHaveText(/^1\s*playlist created$/);
  // Space before and after, from the dry run.
  await expect(view(page).getByTestId("stick-space-line")).toHaveText(/^Free now 25\.8 GB of 34\.4 GB · after this load 25\.\d GB$/);
  // Ready, but the Gig stick is protected: it is unlocked before the hold can be made.
  await expect(loadButton(page)).toBeDisabled();
  await expect(view(page).getByTestId("stick-load-why")).toHaveText("Unlock DJ STICK first.");
  expect(await writes(page)).toEqual([]);
  expect(await call(page, "__stickRecoveringCalls")).toEqual([]);
});

test("a Spare stick is shown as one, and another mounted Stick can be chosen", async ({ page }) => {
  await open(page);
  await call(page, "__setStickSpare", TEST, true);
  await tab(page, "Stick").click();
  await view(page).getByRole("group", { name: "Mounted Sticks" }).getByRole("button", { name: "TEST" }).click();
  await expect(view(page).getByTestId("stick-name")).toHaveText("TEST");
  await expect(view(page).getByTestId("stick-kind")).toHaveText("Spare stick");
  await expect(loadButton(page)).toContainText("hold 1 s · Spare stick");
});

test("the DJ chooses the playlists, the summary follows, and every change opens from it", async ({ page }) => {
  await openStick(page);
  const boxes = playlistRows(page).getByRole("checkbox");
  await boxes.nth(0).check();
  await expect(view(page).getByTestId("stick-summary").locator('[data-change="playlists"]')).toHaveText(/^1\s*playlist created$/);
  await boxes.nth(1).check();
  await expect(view(page).getByTestId("stick-summary").locator('[data-change="playlists"]')).toHaveText(/^2\s*playlists created$/);
  const asked = await call<{ playlists: string[] }>(page, "__stickLoadLastDryRun");
  expect(asked.playlists).toHaveLength(2);

  const show = view(page).getByRole("button", { name: "Show every change" });
  await show.click();
  const changes = view(page).getByRole("region", { name: "Every change" });
  await expect(changes).toBeVisible();
  await expect(changes.getByRole("heading", { name: /^Tracks added \(\d+\)$/ })).toBeVisible();
  await expect(changes.getByRole("heading", { name: "Playlists created (2)" })).toBeVisible();
  const added = Number(/\((\d+)\)/.exec(await changes.getByRole("heading", { name: /^Tracks added/ }).innerText())?.[1]);
  await expect(changes.getByRole("heading", { name: /^Tracks added/ }).locator("xpath=..").getByRole("listitem")).toHaveCount(added);
  await expect(view(page).getByTestId("stick-summary").locator('[data-change="added"]')).toContainText(`+${added}`);

  // Unticking one takes it out of the next dry run.
  await boxes.nth(1).uncheck();
  await expect(view(page).getByTestId("stick-summary").locator('[data-change="playlists"]')).toHaveText(/^1\s*playlist created$/);
  expect(await writes(page)).toEqual([]);
});

test("a Stick with too little space says by how much and the load is disabled", async ({ page }) => {
  await open(page);
  await call(page, "__setStickFree", DJ, 1_000_000);
  await tab(page, "Stick").click();
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-shortfall")).toHaveText(/^Not enough space: this load needs \d+(\.\d)? (MB|GB) more than the Stick has free\.$/);
  await expect(loadButton(page)).toBeDisabled();
  await expect(view(page).getByTestId("stick-load-why")).toContainText("Not enough space");
  expect(await writes(page)).toEqual([]);
});

test("with no Stick mounted the view says so", async ({ page }) => {
  await open(page);
  await page.evaluate(([a, b]) => {
    const hooks = window as unknown as { __unmountDevice: (path: string) => void };
    hooks.__unmountDevice(a!);
    hooks.__unmountDevice(b!);
    window.dispatchEvent(new Event("focus"));
  }, [DJ, TEST]);
  await expect(page.getByTestId("shell-tree").getByRole("group", { name: "Stick TEST" })).toHaveCount(0);
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-none")).toContainText("No Stick is mounted");
});

test("a load needs the Gig stick unlocked by name and the full hold; the Stick is read again after it", async ({ page }) => {
  await openStick(page);
  const name = await chooseFirst(page);
  // Protected: no hold can be made, and nothing is written.
  await expect(loadButton(page)).toBeDisabled();
  expect(await writes(page)).toEqual([]);
  expect(await call(page, "__stickProtected", DJ)).toBe(true);

  await holdToLoad(page);
  await expect(view(page).getByTestId("stick-outcome")).toHaveAttribute("data-outcome", "loaded");
  await expect(view(page).getByTestId("stick-outcome")).toContainText(/Loaded DJ STICK\..*DJ STICK now holds \d+ tracks in 1 playlist/);
  expect((await writes(page)).map((w) => [w.method, w.path])).toEqual([["stickViewLoad", DJ]]);
  expect((await call<{ playlists: string[] }>(page, "__stickHeld", DJ)).playlists).toEqual([name]);
  // No dialog of the gate's: the hold was the view's own.
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  // Read again after the load: unlocked, last loaded today, nothing left to do.
  await expect(view(page).getByTestId("stick-protection")).toHaveText("Unlocked");
  await expect(view(page).getByTestId("stick-meta")).toContainText(/Last loaded [A-Z][a-z]{2} \d\d/);
  await expect(view(page).getByTestId("stick-summary")).toHaveText(/Nothing to change/);
});

test("a load without the hold is refused by the backend and writes nothing", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  await call(page, "__setStickUnlocked", DJ, true);
  const last = await call<{ stick: string; playlists: string[] }>(page, "__stickLoadLastDryRun");
  // A script calling the load straight, with the dry run's plan but no token.
  const refused = await page.evaluate(async ([stick, playlists]) => {
    const write = (window as unknown as { __stickWriteUnstaged: (name: string, ...a: unknown[]) => Promise<unknown> }).__stickWriteUnstaged;
    try {
      await write("stickViewLoad", stick, playlists, "mock-load-1");
      return "loaded";
    } catch (error) {
      return (error as Error).message;
    }
  }, [last.stick, last.playlists] as const);
  expect(refused).toContain("This write to the Stick was not confirmed");
  expect(await writes(page)).toEqual([]);
});

test("a Stick the service cannot plan for says why, and a failed dry run is shown, not the one before", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();

  await call(page, "__failStickDryRuns", "The Stick could not be read.");
  await playlistRows(page).nth(1).getByRole("checkbox").check();
  await expect(view(page).getByTestId("stick-dry-run-failed")).toContainText("The Stick could not be read.");
  await expect(view(page).getByTestId("stick-summary")).toHaveCount(0);
  await expect(loadButton(page)).toBeDisabled();

  await call(page, "__failStickDryRuns", null);
  await call(page, "__setStickNotLoadable", DJ, "Its database is in WAL mode, so what the export would copy cannot be read without writing to it.");
  await view(page).getByRole("button", { name: "Try again" }).click();
  await expect(view(page).getByTestId("stick-not-loadable")).toHaveText(/^This Stick cannot be loaded: Its database is in WAL mode/);
  await expect(loadButton(page)).toBeDisabled();
  expect(await writes(page)).toEqual([]);
});

test("a burst of Library changes costs at most two more dry runs", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  await expect.poll(() => dryRuns(page)).toBeGreaterThan(0);
  await call(page, "__holdStickDryRuns");
  const before = await dryRuns(page);
  await call(page, "__emitLibraryChanged", 30);
  await page.waitForTimeout(300);
  await call(page, "__releaseStickDryRuns");
  await page.waitForTimeout(500);
  expect(await dryRuns(page) - before).toBeLessThanOrEqual(2);
  expect(await dryRuns(page) - before).toBeGreaterThanOrEqual(1);
});

test("the view's controls work from the keyboard and Space never reaches the deck", async ({ page }) => {
  await open(page);
  await titles(page).nth(3).dblclick();
  await tab(page, "Stick").click();
  const box = playlistRows(page).first().getByRole("checkbox");
  await box.focus();
  await page.keyboard.press("Space");
  await expect(box).toBeChecked();
  const show = view(page).getByRole("button", { name: "Show every change" });
  await show.focus();
  await page.keyboard.press("Enter");
  await expect(view(page).getByRole("region", { name: "Every change" })).toBeVisible();
  await view(page).getByRole("button", { name: "Hide the changes" }).focus();
  await page.keyboard.press("Space");
  await expect(view(page).getByRole("region", { name: "Every change" })).toHaveCount(0);
  await expect(pauseButton(page)).toHaveCount(0);
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();
});

for (const width of [1280, 900, 640, 436]) {
  test(`at ${width} px the view fits the window without sideways scrolling`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    await openStick(page);
    await chooseFirst(page);
    await expect(view(page).getByTestId("stick-summary")).toBeVisible();
    const fits = await view(page).evaluate((el) => el.scrollWidth <= el.clientWidth + 1);
    expect(fits).toBe(true);
    const box = await loadButton(page).boundingBox();
    expect(box).not.toBeNull();
    expect((box?.x ?? 0) + (box?.width ?? 0)).toBeLessThanOrEqual(width);
    await page.screenshot({ path: `test-results/stick-view-${width}.png` });
  });
}

test("a Stick with an unfinished write says so, is not dry-run or loaded, and its panel offers the named action", async ({ page }) => {
  await open(page);
  await call(page, "__setStickPending", DJ, true);
  await tab(page, "Stick").click();
  await page.getByTestId("shell-tree").getByRole("group", { name: "Stick DJ STICK" }).getByRole("treeitem", { name: "DJ STICK", exact: true }).click();
  await expect(view(page).getByTestId("stick-unfinished")).toContainText("did not finish");
  await expect(loadButton(page)).toBeDisabled();
  await chooseFirst(page);
  await page.waitForTimeout(300);
  expect(await dryRuns(page)).toBe(0);
  await expect(view(page).getByTestId("stick-panel")).toBeVisible();
  expect(await writes(page)).toEqual([]);
  expect(await call(page, "__stickRecovered")).toEqual([]);
});

// ---- Review of wbx-71 -------------------------------------------------

/**
 * Holds for the load, as the DJ does (wbx-72): an earlier load's end state
 * put away, the Stick unlocked in the view (a Gig stick by name), then the
 * big button held for longer than a Gig stick's hold.
 */
async function holdToLoad(page: Page): Promise<void> {
  const done = view(page).getByTestId("stick-finished").getByRole("button", { name: "Done" });
  if (await done.count() > 0) await done.click();
  const dismiss = view(page).getByTestId("stick-dismiss");
  if (await dismiss.count() > 0) await dismiss.click();
  const name = (await view(page).getByTestId("stick-name").innerText()).trim();
  const field = view(page).getByRole("textbox", { name: `Type ${name} to unlock it` });
  if (await field.count() > 0) await field.fill(name);
  const unlock = view(page).getByRole("button", { name: `Unlock ${name}` });
  if (await unlock.count() > 0) await unlock.click();
  await expect(loadButton(page)).toBeEnabled();
  await loadButton(page).hover();
  await page.mouse.down();
  await page.waitForTimeout(2200);
  await page.mouse.up();
}

test("the hold approves exactly the plan shown: an approval for one dry run never confirms another", async ({ page }) => {
  await openStick(page);
  await call(page, "__setStickUnlocked", DJ, true);
  const refused = await page.evaluate(async ([path]) => {
    const call = (window as unknown as { __stickWriteUnstaged: (name: string, ...a: unknown[]) => Promise<unknown> }).__stickWriteUnstaged;
    const ids = ["pl-0-0"];
    const first = await call("stickViewDryRun", path, ids) as { plan: string };
    const fresh = await call("stickViewDryRun", path, ids) as { plan: string };
    const approval = await call("stickWriteApprove", first.plan);
    try {
      await call("stickViewLoad", path, ids, fresh.plan, approval);
      return "loaded";
    } catch (error) {
      return (error as Error).message;
    }
  }, [DJ] as const);
  expect(refused).toContain("another plan");
  expect(await writes(page)).toEqual([]);
  expect((await call<{ playlists: string[] }>(page, "__stickHeld", DJ)).playlists).toEqual([]);
});

test("the load the hold confirms carries the plan the summary showed", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  const shown = await call<string[]>(page, "__stickLoadPlans");
  await holdToLoad(page);
  await expect(view(page).getByTestId("stick-outcome")).toHaveAttribute("data-outcome", "loaded");
  const loads = await call<{ plan: string }[]>(page, "__stickLoadCalls");
  expect(loads.map((l) => l.plan)).toEqual([shown.at(-1)]);
});

test("a failed load says so and why, and the failure is still there after leaving the view", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await call(page, "__failNextStickLoad", DJ, "The Stick was removed during the export.");
  await holdToLoad(page);
  await expect(view(page).getByTestId("stick-load-error")).toContainText("The Stick was removed during the export.");
  await tab(page, "Library").click();
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-load-error")).toContainText("The Stick was removed during the export.");
});

test("an incomplete load lists what was not written and every Native stems set with its reason", async ({ page }) => {
  await open(page);
  await call(page, "__setStickSpare", TEST, true);
  await tab(page, "Stick").click();
  await view(page).getByRole("group", { name: "Mounted Sticks" }).getByRole("button", { name: "TEST" }).click();
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-summary").locator('[data-change="stems"]')).toBeVisible();
  await call(page, "__nextStickLoadIncomplete", TEST, "Its Stems changed since the dry run.");
  await holdToLoad(page);
  const outcome = view(page).getByTestId("stick-outcome");
  await expect(outcome).toHaveAttribute("data-outcome", "incomplete");
  await expect(outcome).toContainText(/Not written: 1 track/);
  const stems = outcome.getByRole("list", { name: "Native stems" });
  await expect(stems.getByRole("listitem").filter({ hasText: "failed" })).toContainText("Its Stems changed since the dry run.");
  await expect(stems.getByRole("listitem").filter({ hasText: "written" }).first()).toBeVisible();
});

test("a load keeps running when the DJ leaves the view or picks another Stick; its result reaches the DJ and never sits beside the other Stick", async ({ page }) => {
  await openStick(page);
  const name = await chooseFirst(page);
  await call(page, "__holdStickLoads");
  await holdToLoad(page);
  await expect(view(page).getByTestId("stick-progress")).toBeVisible();
  await tab(page, "Library").click();
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-progress")).toBeVisible();
  // Shrunk to the status line, so another Stick can be shown.
  await page.keyboard.press("Escape");
  // Another Stick: its own controls, and no word of the first one's result beside them.
  await view(page).getByRole("group", { name: "Mounted Sticks" }).getByRole("button", { name: "TEST" }).click();
  await expect(view(page).getByTestId("stick-name")).toHaveText("TEST");
  await expect(view(page).getByTestId("stick-outcome")).toHaveCount(0);
  await expect(view(page).getByTestId("stick-other-loads")).toContainText("DJ STICK: loading");
  await call(page, "__releaseStickLoads");
  await expect(view(page).getByTestId("stick-other-loads")).toContainText("DJ STICK: loaded");
  await expect(view(page).getByTestId("stick-outcome")).toHaveCount(0);
  await view(page).getByTestId("stick-other-loads").getByRole("button", { name: "Show DJ STICK" }).click();
  await expect(view(page).getByTestId("stick-name")).toHaveText("DJ STICK");
  await expect(view(page).getByTestId("stick-outcome")).toHaveAttribute("data-outcome", "loaded");
  expect((await call<{ playlists: string[] }>(page, "__stickHeld", DJ)).playlists).toEqual([name]);
});

test("a repeat load says nothing needed writing and what the Stick now holds, not that it holds nothing", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await holdToLoad(page);
  await expect(view(page).getByTestId("stick-outcome")).toContainText(/Written: \d+ tracks, 1 playlist/);
  await expect(view(page).getByTestId("stick-summary")).toHaveText(/Nothing to change/);
  await holdToLoad(page);
  const outcome = view(page).getByTestId("stick-outcome");
  await expect(outcome).toContainText("Nothing needed writing");
  await expect(outcome).toContainText(/Already as planned: \d+ tracks, 1 playlist/);
  await expect(outcome).toContainText(/DJ STICK now holds [1-9]\d* tracks in 1 playlist/);
});

test("a playlist whose tracks change is a change, though no track is added to the Stick", async ({ page }) => {
  await open(page);
  // The Stick holds both chosen playlists' tracks, but the first one's last track sits only in the second.
  const ids = ["pl-0-0", "pl-0-1"];
  const tracks = await Promise.all(ids.map((id) => call<string[]>(page, "__playlistTracks", id)));
  const moved = tracks[0]!.at(-1)!;
  await call(page, "__setStickHolds", DJ, [
    { id: ids[0], name: "Melodic Vox", tracks: tracks[0]!.slice(0, -1) },
    { id: ids[1], name: "Hardstyle", tracks: [...tracks[1]!, moved] },
  ]);
  await tab(page, "Stick").click();
  await playlistRows(page).nth(0).getByRole("checkbox").check();
  await playlistRows(page).nth(1).getByRole("checkbox").check();
  const summary = view(page).getByTestId("stick-summary");
  await expect(summary.locator('[data-change="membership"]')).toContainText("Melodic Vox (+1)");
  await expect(summary).not.toContainText("Nothing to change");
});

test("a Stick replaced at the same place drops its preview at once, and a late answer for the old one is not shown", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await expect(view(page).getByTestId("stick-space-line")).toContainText("after this load");
  await call(page, "__holdStickDryRuns");
  // Something else ticks a second playlist: a dry run for the old Stick is now in flight.
  await playlistRows(page).nth(1).getByRole("checkbox").check();
  await expect(view(page).getByTestId("stick-summary-reading")).toBeVisible();
  // Another Stick is now mounted at the same path.
  await call(page, "__swapStickAt", DJ, 2_000_000_000);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(view(page).getByTestId("stick-summary")).toHaveCount(0);
  await expect(view(page).getByTestId("stick-load")).toBeDisabled();
  // The old Stick's answer arrives late: not shown.
  await call(page, "__releaseFirstStickDryRun");
  await page.waitForTimeout(300);
  await expect(view(page).getByTestId("stick-summary")).toHaveCount(0);
  await call(page, "__releaseStickDryRuns");
  await expect(view(page).getByTestId("stick-space-line")).toContainText("Free now 2.0 GB");
});

test("after a Stick is chosen with the buttons, choosing one in the tree still shows it", async ({ page }) => {
  await openStick(page);
  await view(page).getByRole("group", { name: "Mounted Sticks" }).getByRole("button", { name: "TEST" }).click();
  await expect(view(page).getByTestId("stick-name")).toHaveText("TEST");
  await page.getByTestId("shell-tree").getByRole("group", { name: "Stick DJ STICK" }).getByRole("treeitem", { name: "DJ STICK", exact: true }).click();
  await expect(view(page).getByTestId("stick-name")).toHaveText("DJ STICK");
});

// ---- Reports kept across a close (wbx-71 verification review) ----------

const KEPT = "rbl.mock.stickLoadReports";

test("a load report kept when the window closed is shown at the next start, with its Stick not mounted, until dismissed", async ({ page }) => {
  const kept = {
    id: "op-closed-during-copying",
    stick: { path: "/Volumes/OLD GIG", name: "OLD GIG", volume: "dev 9 ino 9" },
    finished: "2026-11-13 21:04:00.000 +00:00",
    report: {
      stick: { path: "/Volumes/OLD GIG", name: "OLD GIG", kind: "gig", protected: false, tracks: null, playlists: null },
      outcome: "failed",
      error: { code: "service_failed", message: "the load stopped because the Library service was stopped" },
      written: { tracks: [], playlists: [] },
      not_written: { tracks: [{ id: "1", title: "Alpha" }, { id: "2", title: "Bravo" }], playlists: ["Set"] },
      unchanged: { tracks: [], playlists: [] },
      native_stems: [{ track: { id: "1", title: "Alpha" }, outcome: "not_written", reason: "the export failed first", stems_fact: null }],
    },
    error: null,
  };
  await page.addInitScript(([key, record]) => {
    if (sessionStorage.getItem(`${key}.seeded`) === null) {
      sessionStorage.setItem(key, JSON.stringify([record]));
      sessionStorage.setItem(`${key}.seeded`, "1");
    }
  }, [KEPT, kept] as const);
  await open(page);
  await page.evaluate(([a, b]) => {
    const hooks = window as unknown as { __unmountDevice: (path: string) => void };
    hooks.__unmountDevice(a!);
    hooks.__unmountDevice(b!);
    window.dispatchEvent(new Event("focus"));
  }, [DJ, TEST]);
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-none")).toBeVisible();

  const earlier = view(page).getByTestId("stick-kept-reports");
  await expect(earlier).toContainText("OLD GIG (not mounted)");
  await expect(earlier).toContainText("The load failed");
  await expect(earlier).toContainText("the load stopped because the Library service was stopped");
  await expect(earlier).toContainText("Nothing was written");
  await expect(earlier).toContainText("Not written: 2 tracks, 1 playlist");
  await expect(earlier.getByRole("list", { name: "Native stems" })).toContainText("Alpha: not_written (the export failed first)");

  await earlier.getByRole("button", { name: "Dismiss the report for OLD GIG" }).click();
  await expect(view(page).getByTestId("stick-kept-reports")).toHaveCount(0);
  await page.reload();
  await tab(page, "Stick").click();
  await expect(view(page).getByTestId("stick-name")).toBeVisible();
  await expect(view(page).getByTestId("stick-kept-reports")).toHaveCount(0);
});

test("a failed load is kept: after a reload it is still reported until dismissed; a completed one is not kept twice", async ({ page }) => {
  await openStick(page);
  await chooseFirst(page);
  await holdToLoad(page);
  await expect(view(page).getByTestId("stick-outcome")).toHaveAttribute("data-outcome", "loaded");
  await expect.poll(() => page.evaluate((key) => (JSON.parse(sessionStorage.getItem(key) ?? "[]") as unknown[]).length, KEPT)).toBe(0);

  await call(page, "__failNextStickLoad", DJ, "The Stick was removed during the export.");
  await playlistRows(page).nth(1).getByRole("checkbox").check();
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
  await holdToLoad(page);
  await expect(view(page).getByTestId("stick-load-error")).toContainText("The Stick was removed during the export.");
  await page.reload();
  await tab(page, "Stick").click();
  const earlier = view(page).getByTestId("stick-kept-reports");
  await expect(earlier).toContainText("DJ STICK");
  await expect(earlier).toContainText("The Stick was removed during the export.");
  await earlier.getByRole("button", { name: "Dismiss the report for DJ STICK" }).click();
  await expect(view(page).getByTestId("stick-kept-reports")).toHaveCount(0);
});
