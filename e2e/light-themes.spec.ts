/**
 * Light Omarchy themes recolour the new views (wbx-45.1).
 *
 * A light theme is announced to the mock interface the way the app shell
 * announces one (`followTheme()` hears the event). What is read back is what
 * real controls in each view are drawn in: their text and icons against the
 * ground actually painted behind them (`lightAudit.ts`), the grounds of the
 * shell's regions, and the colours that carry meaning.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { audit, shortfalls, type Seen } from "./lightAudit";
import { load } from "./compactPlayer";
import { LATTE, WHITE, TOKYO_NIGHT, announce, rgb } from "./lightPalettes";

const VIEWS = ["Library", "Intake", "Review", "Stick"] as const;

/** Colours that carry meaning: bands, phrases, cues, Prep colours, key lights, Energy, meters, Mutant magenta. */
const MEANING = [
  "--c-wave-low", "--c-wave-mid", "--c-wave-high", "--c-wave-all", "--c-phrase-intro", "--c-phrase-chorus", "--c-phrase-up",
  "--c-cue-hot", "--c-comment-pink", "--c-comment-yellow", "--c-comment-green", "--c-energy-0", "--c-energy-1",
  "--c-energy-3", "--c-energy-5", "--c-meter-green", "--c-meter-red", "--c-playhead", "--c-loop-band",
];

/** Each token as the browser resolves it inside the shell, so `var()` chains and scoped overrides are followed. */
async function resolved(page: Page, names: string[]): Promise<Record<string, string>> {
  return page.getByTestId("shell-working").evaluate((at, tokens) => {
    const probe = document.createElement("i");
    at.append(probe);
    const found: Record<string, string> = {};
    for (const name of tokens) {
      probe.style.color = `var(${name})`;
      found[name] = getComputedStyle(probe).color;
    }
    probe.remove();
    return found;
  }, names);
}

const ground = (target: Locator): Promise<string> => target.evaluate((el) => getComputedStyle(el).backgroundColor);

async function showView(page: Page, view: (typeof VIEWS)[number]): Promise<void> {
  const tab = page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: view });
  await tab.click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
  if (view === "Intake") {
    // A queued track chosen, so its track and tag panels are drawn.
    const first = page.getByRole("listbox", { name: "Intake queue" }).getByRole("option").first();
    await expect(first).toBeVisible();
    if (await first.getAttribute("aria-selected") !== "true") await first.click();
    await expect(page.getByRole("region", { name: "Tags" }).getByRole("group").first()).toBeVisible();
  }
}

/** The Library with a track loaded and a row selected, so the player, the selection and the queue all have something in them. */
async function openShell(page: Page): Promise<void> {
  await load(page);
  await page.locator('[role="gridcell"][data-col="title"]').nth(1).click();
  // The second list beside the first.
  await page.keyboard.press("Control+Backslash");
  await expect(page.getByRole("region", { name: "Second list" })).toBeVisible();
}

test.describe.configure({ timeout: 120_000 });

test("under a light theme each view's text and icons can be read on the ground behind them", async ({ page }) => {
  await openShell(page);
  for (const roles of [LATTE, WHITE]) {
    await announce(page, { roles, font: null });
    await expect(page.locator("html")).toHaveAttribute("data-theme-mode", "light");
    for (const view of VIEWS) {
      await showView(page, view);
      const { seen } = await audit(page.getByTestId("shell"));
      expect(seen.length, view).toBeGreaterThan(10);
      expect([...new Set(shortfalls(seen))], `${roles.working} ${view}`).toEqual([]);
    }
    await showView(page, "Library");
  }
});

test("under a light theme the Stick view with a Stick selected, a load chosen and its settings open can be read", async ({ page }) => {
  await openShell(page);
  const stick = page.getByRole("tree", { name: "Tree" }).getByRole("group", { name: "Stick TEST" });
  await stick.getByRole("treeitem", { name: "TEST", exact: true }).click();
  const view = page.getByTestId("stick-view");
  await expect(view.getByTestId("stick-name")).toHaveText("TEST");
  // A playlist chosen, so the load's summary (its counts and the space it adds) is drawn.
  await view.getByRole("button", { name: /^Choose playlists/ }).click();
  await view.getByTestId("stick-choose").getByRole("checkbox", { name: "Melodic Vox" }).check();
  await expect(view.getByTestId("stick-summary")).toBeVisible();
  // Upstream's device panel inside it.
  await view.getByRole("button", { name: "TEST's settings and tools" }).click();
  await expect(view.getByTestId("stick-panel")).toBeVisible();
  for (const roles of [LATTE, WHITE]) {
    await announce(page, { roles, font: null });
    const { seen } = await audit(page.getByTestId("shell"));
    expect([...new Set(shortfalls(seen))], roles.working).toEqual([]);
  }
});

test("under a light theme the device panel's image buttons are drawn as buttons, not empty squares", async ({ page }) => {
  await openShell(page);
  const stick = page.getByRole("tree", { name: "Tree" }).getByRole("group", { name: "Stick TEST" });
  await stick.getByRole("treeitem", { name: "TEST", exact: true }).click();
  await page.getByTestId("stick-view").getByRole("button", { name: "TEST's settings and tools" }).click();
  const buttons = page.getByTestId("stick-panel").getByRole("button", { name: /^(Copy from \[Preferences\]|Import image file|Delete image)$/ });
  await expect(buttons).toHaveCount(3);
  for (const roles of [LATTE, WHITE]) {
    await announce(page, { roles, font: null });
    for (const button of await buttons.all()) {
      // The glyph square beside each label: its fill or its edge stands out from the ground behind it.
      const stands = await button.evaluate((el) => {
        const glyph = el.querySelector("[aria-hidden]")!;
        const parse = (c: string) => (c.match(/[\d.]+/g) ?? []).map(Number);
        const lum = (c: number[]) => {
          const [r, g, b] = c.slice(0, 3).map((v) => { const x = v / 255; return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4; });
          return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
        };
        const ratio = (a: number[], b: number[]) => { const [h, l] = [lum(a), lum(b)].sort((x, y) => y - x); return (h! + 0.05) / (l! + 0.05); };
        let behind = [255, 255, 255];
        for (let at = glyph.parentElement; at; at = at.parentElement) {
          const c = parse(getComputedStyle(at).backgroundColor);
          if (c.length >= 3 && (c[3] ?? 1) > 0.5) { behind = c; break; }
        }
        const s = getComputedStyle(glyph);
        const edge = s.borderTopStyle !== "none" && parseFloat(s.borderTopWidth) > 0 ? ratio(parse(s.borderTopColor), behind) : 0;
        return Math.max(ratio(parse(s.backgroundColor), behind), edge);
      });
      expect(stands, `${await button.textContent()} under ${roles.working}`).toBeGreaterThanOrEqual(3);
    }
  }
});

test("under a light theme every view is drawn on the theme's own grounds, with its text, selection and accent", async ({ page }) => {
  await openShell(page);
  await announce(page, { roles: LATTE, font: null });
  const themeGrounds = new Set([LATTE.ground, LATTE.panel, LATTE.working, LATTE.hairline, LATTE.selection].map(rgb));
  const meaning = new Set(Object.values(await resolved(page, [
    ...MEANING, "--c-phrase-verse", "--c-phrase-bridge", "--c-phrase-up2", "--c-phrase-up3", "--c-phrase-down", "--c-phrase-outro",
    "--c-energy-2", "--c-energy-4",
  ])).concat([rgb("#9ece6a"), rgb("#e0af68")])); // the key lights' fixed fills (LibraryKey.module.css)
  const allowed = (s: Seen) => s.well || s.meaningGround || themeGrounds.has(s.ground) || meaning.has(s.ground)
    // An accent-filled control, or a lit one drawn in the text colour with the ground's colour on it.
    || s.ground === rgb(LATTE.accent) || s.ground === rgb(LATTE.text);

  // The regions of the frame: no dark panel left behind.
  for (const region of ["shell-top-bar", "shell-tree", "shell-working", "shell-status", "shell-player"]) {
    const regionGround = await page.getByTestId(region).evaluate((el) => {
      for (let at: Element | null = el; at; at = at.parentElement) {
        const c = getComputedStyle(at).backgroundColor;
        if (c !== "rgba(0, 0, 0, 0)") return c;
      }
      return "";
    });
    expect(themeGrounds.has(regionGround), `${region}: ${regionGround}`).toBe(true);
  }
  for (const view of VIEWS) {
    await showView(page, view);
    const { seen } = await audit(page.getByTestId("shell"));
    const strays = [...new Set(seen.filter((s) => s.kind !== "placeholder" && !allowed(s)).map((s) => `${s.what} on ${s.ground}`))];
    expect(strays, view).toEqual([]);
  }

  // Text, selection and the accent are the theme's.
  await showView(page, "Library");
  const tree = page.getByRole("tree", { name: "Tree" });
  const selected = tree.getByRole("treeitem", { selected: true }).first();
  expect(await selected.evaluate((el) => getComputedStyle(el).color)).toBe(rgb(LATTE.text));
  expect(await ground(selected)).toBe(rgb(LATTE.selection));
  const row = page.getByRole("tabpanel").locator('[role="row"][aria-selected="true"]').first();
  expect(await ground(row)).toBe(rgb(LATTE.selection));
  const activeTab = page.getByRole("tablist", { name: "Views" }).getByRole("tab", { selected: true });
  expect(await activeTab.evaluate((el) => getComputedStyle(el).borderImageSource))
    .toBe(`linear-gradient(90deg, ${rgb(LATTE.tabFrom)}, ${rgb(LATTE.tabTo)})`);
  await page.getByRole("textbox", { name: "Filter text" }).focus();
  await page.keyboard.press("Tab");
  const focused = page.locator(":focus-visible");
  const ring = await focused.evaluate((el) => {
    const s = getComputedStyle(el);
    return s.outlineStyle !== "none" ? s.outlineColor : s.boxShadow;
  });
  // The theme's accent, or the same hue darkened until it can be written with.
  const [r, g, b] = (ring.match(/rgb\((\d+), (\d+), (\d+)\)/) ?? []).slice(1).map(Number);
  const accent = [1, 3, 5].map((at) => Number.parseInt(LATTE.accent.slice(at, at + 2), 16));
  const factor = b! / accent[2]!;
  expect(factor, ring).toBeGreaterThan(0.5);
  expect(factor, ring).toBeLessThanOrEqual(1);
  expect(Math.abs(r! - accent[0]! * factor), ring).toBeLessThanOrEqual(2);
  expect(Math.abs(g! - accent[1]! * factor), ring).toBeLessThanOrEqual(2);
});

test("colours that carry meaning are the same under a light theme as under a dark one, and each stands out from its light ground", async ({ page }) => {
  await openShell(page);
  const list = page.getByRole("tabpanel");
  const marks = {
    pip: list.locator('[role="radiogroup"][aria-label^="Energy"] [role="radio"][aria-checked="true"]').first(),
    stripe: list.getByRole("img", { name: /^Prep colour: / }).first(),
    key: list.locator('[data-light="match"], [data-light="boost"]').first(),
    phrase: page.getByTestId("compact-player-phrases").locator("[data-phrase]").first(),
  };
  const fills = async () => {
    const found: Record<string, string> = {};
    for (const [name, mark] of Object.entries(marks)) found[name] = await ground(mark);
    return { ...found, ...(await resolved(page, MEANING)) };
  };
  await announce(page, { roles: TOKYO_NIGHT, font: null });
  const dark = await fills();
  for (const roles of [LATTE, WHITE]) {
    await announce(page, { roles, font: null });
    expect(await fills(), roles.working).toEqual(dark);
    for (const [name, mark] of Object.entries(marks)) {
      const stands = await mark.evaluate((el) => {
        type RGB = number[];
        const parse = (c: string): RGB => (c.match(/[\d.]+/g) ?? []).map(Number);
        const lum = (c: RGB) => {
          const [r, g, b] = c.slice(0, 3).map((v) => { const x = v / 255; return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4; });
          return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
        };
        const ratio = (a: RGB, b: RGB) => { const [h, l] = [lum(a), lum(b)].sort((x, y) => y - x); return (h! + 0.05) / (l! + 0.05); };
        let behind: RGB = [255, 255, 255];
        for (let at = el.parentElement; at; at = at.parentElement) {
          const c = parse(getComputedStyle(at).backgroundColor);
          if (c.length >= 3 && (c[3] ?? 1) > 0.5) { behind = c; break; }
        }
        const s = getComputedStyle(el);
        const own = ratio(parse(s.backgroundColor), behind);
        // Where the colour itself is too close to the ground, an outline or well must carry it.
        const edges = [s.boxShadow, s.outlineStyle !== "none" ? s.outlineColor : "", s.borderTopStyle !== "none" ? s.borderTopColor : ""]
          .flatMap((v) => v.match(/rgba?\([^)]*\)/g) ?? [])
          .map(parse)
          .filter((c) => (c[3] ?? 1) > 0.5);
        const edge = Math.max(0, ...edges.map((c) => ratio(c, behind)));
        return { own, edge, best: Math.max(own, edge) };
      });
      expect(stands.best, `${name} under ${roles.working}: ${JSON.stringify(stands)}`).toBeGreaterThanOrEqual(3);
    }
  }
});

test("switching between a dark and a light theme while open changes every view without a reload", async ({ page }) => {
  await openShell(page);
  const working = page.getByTestId("shell-working");
  const marker = await page.evaluate(() => ((window as unknown as { __noReload: number }).__noReload = Math.random()));
  for (const view of VIEWS) {
    await showView(page, view);
    await announce(page, { roles: TOKYO_NIGHT, font: null });
    const tree = page.getByRole("tree", { name: "Tree" }).getByRole("treeitem").first();
    expect(await tree.evaluate((el) => getComputedStyle(el).color), view).toBe(rgb(TOKYO_NIGHT.text));
    await announce(page, { roles: LATTE, font: null });
    expect(await tree.evaluate((el) => getComputedStyle(el).color), view).toBe(rgb(LATTE.text));
    const { seen } = await audit(working);
    expect(seen.some((s) => [LATTE.ground, LATTE.panel, LATTE.working].map(rgb).includes(s.ground)), view).toBe(true);
    await announce(page, { roles: TOKYO_NIGHT, font: null });
    const after = await audit(working);
    expect(after.seen.some((s) => [LATTE.ground, LATTE.panel, LATTE.working].map(rgb).includes(s.ground)), view).toBe(false);
  }
  expect(await page.evaluate(() => (window as unknown as { __noReload: number }).__noReload)).toBe(marker);
});
