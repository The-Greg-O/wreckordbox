/**
 * Every state of the Stick view reads under a light theme (wbx-4g6): before
 * loading (with the in-place unlock and its name field, then unlocked),
 * holding (the ring and its hint), loading with its bar, shrunk to the
 * status line, failed with its report, kept reports from an earlier start,
 * finished with Eject, and safe to remove. Under Catppuccin Latte and White,
 * at the 0.8, 1 and 1.25 scales.
 *
 * What is read back is what is painted: text at 4.5:1 and secondary text at
 * 3:1 against the ground behind it (`lightAudit.ts`), and 3:1 for what
 * carries meaning without words: the hold ring's fill against its track and
 * the button, the progress bars' fill against their track and the ground,
 * and the coloured edges of the finished, failed, kept and safe-to-remove
 * panels against the ground around them (by their edge, or the outline a
 * light theme gives a meaning colour).
 *
 * `STICK_SHOTS=<dir>` also saves a screenshot of each state under Latte,
 * White and Tokyo Night at each scale (for wbx-4g6's before/after check of
 * the dark themes).
 */
import { expect, test, type Locator, type Page } from "@playwright/test";

import { open } from "./compactPlayer";
import { audit, shortfalls, type Seen } from "./lightAudit";
import { LATTE, TOKYO_NIGHT, WHITE, announce } from "./lightPalettes";
import type { ThemeRoles } from "../src/lib/themeRoles";

test.describe.configure({ timeout: 420_000 });

const DJ = "/Volumes/DJ STICK";
const LIGHT = [{ name: "latte", roles: LATTE }, { name: "white", roles: WHITE }] as const;
const DARK = { name: "tokyo-night", roles: TOKYO_NIGHT } as const;
const SHOTS = process.env.STICK_SHOTS;

const view = (page: Page) => page.getByTestId("stick-view");
const holdButton = (page: Page) => view(page).getByTestId("stick-load");
const statusLoad = (page: Page) => page.getByTestId("status-stick-load");

async function call<T>(page: Page, name: string, ...args: unknown[]): Promise<T> {
  return page.evaluate(([hook, rest]) => {
    const hooks = window as unknown as Record<string, (...a: unknown[]) => T>;
    return hooks[hook]!(...rest);
  }, [name, args] as [string, unknown[]]);
}

type Part = "ring" | "bar" | "edge";

/**
 * The contrasts that make a part readable without words, as painted:
 * `ring` the hold ring's fill against its track and the button's ground;
 * `bar` a progress bar's fill against its track and the ground around it;
 * `edge` a panel's edge (its border, or the outline a light theme adds)
 * against the ground around it.
 */
const measure = (part: Locator, kind: Part): Promise<Record<string, number>> => part.evaluate((el, kind) => {
  type C = [number, number, number, number];
  const parse = (css: string): C => {
    const m = /^(rgba?|color)\((?:srgb\s+)?([^)]+)\)/.exec(css.trim());
    if (!m) return [0, 0, 0, 0];
    const [r, g, b, a] = m[2]!.split(/[\s,/]+/).filter(Boolean).map(Number);
    const k = m[1] === "color" ? 255 : 1;
    return [r! * k, g! * k, b! * k, a ?? 1];
  };
  const over = (top: C, under: C): C => {
    const a = top[3];
    return [top[0] * a + under[0] * (1 - a), top[1] * a + under[1] * (1 - a), top[2] * a + under[2] * (1 - a), 1];
  };
  const lum = (c: C) => {
    const [r, g, b] = c.slice(0, 3).map((v) => { const s = v / 255; return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; });
    return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
  };
  const contrast = (a: C, b: C) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x! + 0.05) / (y! + 0.05); };
  /** The ground painted at `from` and behind it, translucent layers composited. */
  const groundOf = (from: Element | null): C => {
    const layers: C[] = [];
    for (let at = from; at; at = at.parentElement) {
      const c = parse(getComputedStyle(at).backgroundColor);
      if (c[3] > 0) { layers.push(c); if (c[3] >= 1) break; }
    }
    let result: C = layers.length && layers[layers.length - 1]![3] >= 1 ? layers.pop()! : [255, 255, 255, 1];
    while (layers.length) result = over(layers.pop()!, result);
    return result;
  };
  if (kind === "ring") {
    const [track, fill] = [...el.querySelectorAll("svg circle")];
    const ground = groundOf(el);
    const fillColour = over(parse(getComputedStyle(fill!).stroke), ground);
    const trackColour = over(parse(getComputedStyle(track!).stroke), ground);
    return { "fill on track": contrast(fillColour, trackColour), "fill on the button": contrast(fillColour, ground) };
  }
  if (kind === "bar") {
    const ground = groundOf(el.parentElement);
    const track = over(parse(getComputedStyle(el).backgroundColor), ground);
    const fillEl = el.firstElementChild!;
    const fill = over(parse(getComputedStyle(fillEl).backgroundColor), track);
    return { "fill on track": contrast(fill, track), "fill on the ground": contrast(fill, ground), width: fillEl.getBoundingClientRect().width };
  }
  const style = getComputedStyle(el);
  const around = groundOf(el.parentElement);
  const width = (el as HTMLElement).offsetWidth - (el as HTMLElement).clientWidth;
  const border = width >= 1 ? contrast(over(parse(style.borderTopColor), around), around) : 1;
  const shadow = style.boxShadow === "none" ? 1 : contrast(over(parse(style.boxShadow), around), around);
  return { edge: Math.max(border, shadow) };
}, kind);

type Check = { what: string; part: Locator; kind: Part };

/** Under each light theme: every text in the shell and each part that carries meaning; then the shots. */
async function readable(page: Page, state: string, scale: number, parts: Check[] = [], region?: Locator): Promise<void> {
  for (const theme of LIGHT) {
    await announce(page, { roles: theme.roles, font: null });
    await readUnder(page, state, scale, theme.name, theme.roles, parts, region);
  }
  await shot(page, state, scale, DARK.name, DARK.roles);
}

async function readUnder(page: Page, state: string, scale: number, name: string, roles: ThemeRoles, parts: Check[], region?: Locator): Promise<void> {
  const where = `${state}, ${name}, scale ${scale}`;
  // The view, and the status line, where a running or failed load is also shown.
  const seen: Seen[] = [];
  for (const at of region ? [region] : [view(page), page.getByTestId("status-line")]) seen.push(...(await audit(at)).seen);
  expect.soft([...new Set(shortfalls(seen))], where).toEqual([]);
  for (const { what, part, kind } of parts) {
    const found = await measure(part, kind);
    for (const [key, value] of Object.entries(found)) {
      if (key === "width") expect.soft(value, `${where}: ${what} shows some progress`).toBeGreaterThan(0);
      else expect.soft(value, `${where}: ${what}, ${key}`).toBeGreaterThanOrEqual(3);
    }
  }
  if (roles !== DARK.roles) await save(page, state, scale, name);
}

async function shot(page: Page, state: string, scale: number, name: string, roles: ThemeRoles): Promise<void> {
  if (!SHOTS) return;
  await announce(page, { roles, font: null });
  await save(page, state, scale, name);
}

async function save(page: Page, state: string, scale: number, name: string): Promise<void> {
  if (!SHOTS) return;
  await page.waitForTimeout(150);
  await page.screenshot({ path: `${SHOTS}/${state}-${name}-${scale}.png`, animations: "disabled" });
}

/** The Gig stick DJ STICK in the Stick view, its first playlist chosen. */
async function chosen(page: Page): Promise<void> {
  await page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "Stick" }).click();
  await expect(view(page).getByTestId("stick-name")).toHaveText("DJ STICK");
  const box = view(page).getByTestId("stick-choose").locator("label").filter({ hasNotText: "(folder" }).first().getByRole("checkbox");
  // After a reload the choice is remembered and the list stays closed.
  await expect(view(page).getByTestId("stick-summary").or(box)).toBeVisible();
  if (await box.isVisible() && !await box.isChecked()) await box.check();
  await expect(view(page).getByTestId("stick-summary")).toBeVisible();
}

async function unlock(page: Page): Promise<void> {
  await view(page).getByRole("textbox", { name: "Type DJ STICK to unlock it" }).fill("DJ STICK");
  await view(page).getByRole("button", { name: "Unlock DJ STICK" }).click();
  await expect(view(page).getByTestId("stick-protection")).toHaveText("Unlocked");
  await expect(holdButton(page)).toBeEnabled();
}

async function pressFor(page: Page, ms: number): Promise<void> {
  await holdButton(page).hover();
  await page.mouse.down();
  await page.waitForTimeout(ms);
  await page.mouse.up();
}

const ring = (page: Page): Check => ({ what: "the hold ring", part: holdButton(page), kind: "ring" });

for (const [stop, scale] of [[2, 0.8], [4, 1], [6, 1.25]] as const) {
  test(`at scale ${scale} every state of the Stick view reads under Latte and White`, async ({ page }) => {
    await open(page, "", { uiScale: stop });
    await chosen(page);

    // Before loading: protected, the in-place unlock with its name field.
    await expect(view(page).getByRole("textbox", { name: "Type DJ STICK to unlock it" })).toBeVisible();
    await readable(page, "unlock", scale);
    await unlock(page);
    await readable(page, "before-loading", scale, [ring(page)]);

    // Holding: the ring filling and its hint. Each hold is let go of well before it is full (2 s), so nothing loads.
    const hold = holdButton(page).locator("xpath=..");
    for (const theme of [...LIGHT, DARK]) {
      await announce(page, { roles: theme.roles, font: null });
      await holdButton(page).hover();
      await page.mouse.down();
      await expect(holdButton(page)).toHaveAttribute("data-holding");
      await page.waitForTimeout(600);
      if (theme.roles === DARK.roles) await save(page, "holding", scale, theme.name);
      else await readUnder(page, "holding", scale, theme.name, theme.roles, [ring(page)], hold);
      await page.mouse.up();
      await page.waitForTimeout(300);
    }
    expect(await call<unknown[]>(page, "__stickLoadCalls"), "no hold was full").toEqual([]);

    // Failed, with its report.
    await call(page, "__failNextStickLoad", DJ, "The Stick was removed during the export.");
    await pressFor(page, 2300);
    const failed = view(page).getByTestId("stick-failed");
    await expect(failed).toContainText("The Stick was removed during the export.");
    await readable(page, "failed", scale, [{ what: "the failed panel's edge", part: failed, kind: "edge" }]);

    // Kept reports from an earlier start: the window opened again.
    await page.reload();
    await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
    await chosen(page);
    const kept = view(page).getByTestId("stick-kept-reports");
    await expect(kept).toContainText("The Stick was removed during the export.");
    await readable(page, "kept-reports", scale, [{ what: "the kept reports' edge", part: kept, kind: "edge" }]);
    await kept.getByRole("button", { name: "Dismiss the report for DJ STICK" }).click();
    await expect(kept).toHaveCount(0);

    // Loading, with its bar.
    await unlock(page);
    await call(page, "__holdStickLoads");
    await pressFor(page, 2300);
    const progress = view(page).getByTestId("stick-progress");
    await expect(progress).toBeVisible();
    await call(page, "__emitStickLoadProgress", DJ, 3);
    await expect(progress).toContainText("Burst track 3");
    await readable(page, "loading", scale, [
      { what: "the load's bar", part: progress.getByRole("progressbar"), kind: "bar" },
      { what: "the status line's bar", part: statusLoad(page).getByRole("progressbar"), kind: "bar" },
    ]);
    // The loud display over a track's own waveform, and the plain bar the setting gives instead (wbx-73).
    await call(page, "__emitStickLoadTrack", DJ, 2, 3, "100003");
    await expect(progress.getByTestId("stick-loud")).toHaveAttribute("data-wave", "analysis");
    await readable(page, "loading-waveform", scale, [{ what: "the load's bar", part: progress.getByRole("progressbar"), kind: "bar" }]);
    await progress.getByRole("button", { name: "Show a plain bar" }).click();
    await expect(progress.getByTestId("stick-loud")).toHaveCount(0);
    await readable(page, "loading-plain", scale, [{ what: "the plain bar", part: progress.getByRole("progressbar"), kind: "bar" }]);
    await progress.getByRole("button", { name: "Show the loud display" }).click();

    // Shrunk to the status line.
    await page.keyboard.press("Escape");
    await expect(view(page).getByTestId("stick-shrunk")).toBeVisible();
    await readable(page, "shrunk", scale, [
      { what: "the status line's bar", part: statusLoad(page).getByRole("progressbar"), kind: "bar" },
    ]);

    // Finished, with Eject.
    await call(page, "__releaseStickLoads");
    const finished = view(page).getByTestId("stick-finished");
    await expect(finished).toContainText("Loaded DJ STICK");
    await readable(page, "finished", scale, [{ what: "the finished panel's edge", part: finished, kind: "edge" }]);

    // Safe to remove, after the eject.
    await finished.getByRole("button", { name: "Eject DJ STICK" }).click();
    const safe = page.getByTestId("stick-safe-to-remove");
    await expect(safe).toHaveText(/DJ STICK is safe to remove/);
    await readable(page, "safe-to-remove", scale, [{ what: "the safe-to-remove panel's edge", part: safe.locator(".."), kind: "edge" }]);
  });
}
