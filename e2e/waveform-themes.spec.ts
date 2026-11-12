/**
 * Waveform colour themes and sharp waveforms (ticket #48) across every
 * waveform the window has: upstream's deck (overview, detail, the rows'
 * previews), the view shell's compact player overview, and Intake's large
 * player detail.
 *
 * The theme the app shell would send is stood in for with the
 * `wreckordbox:theme` event (`themeRoles.ts`): `wave` is the Omarchy
 * palette's three colours for the Theme waveform, `waveform` the theme
 * file's choice, `roles` the window's chrome.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { player as shellPlayer, seedShell } from "./compactPlayer";

const PINK = [243, 139, 168] as const;
const MEASURED_BLUE = [0, 85, 225] as const;
const WAVE = { low: "#f38ba8", mid: "#fab387", high: "#f9e2af" };

/** Every role of the window's chrome, all in one loud colour. */
const roles = (colour: string) => ({
  mode: "dark", ground: "#000000", panel: "#111111", working: "#222222", hairline: "#333333",
  selection: "#444444", border: "#555555", text: "#dddddd", textDim: "#999999", textBright: colour,
  accent: colour, onAccent: "#000000", tabFrom: colour, tabTo: colour,
});

const showTheme = (page: Page, theme: Record<string, unknown>) =>
  page.evaluate((detail) => window.dispatchEvent(new CustomEvent("wreckordbox:theme", { detail })), { roles: null, font: null, ...theme });

/** Whether a canvas has a pixel of exactly this colour, anywhere. */
const paints = (canvas: Locator, rgb: readonly [number, number, number]) =>
  canvas.evaluate((c: HTMLCanvasElement, [r, g, b]) => {
    const ctx = c.getContext("2d");
    if (!ctx || c.width === 0 || c.height === 0) return false;
    const data = ctx.getImageData(0, 0, c.width, c.height).data;
    for (let i = 0; i < data.length; i += 4) {
      if (data[i] === r && data[i + 1] === g && data[i + 2] === b && data[i + 3] === 255) return true;
    }
    return false;
  }, rgb);

const pixels = (canvas: Locator) => canvas.evaluate((c: HTMLCanvasElement) =>
  Array.from(c.getContext("2d")?.getImageData(0, 0, c.width, c.height).data ?? []).join(","));

const deck = (page: Page) => page.getByRole("region", { name: "Preview player" });
const deckOverview = (page: Page) => page.getByTestId("player-overview").locator("canvas").first();

async function openDeck(page: Page, view: Record<string, unknown> = {}) {
  await page.addInitScript((extra) => {
    if (sessionStorage.getItem("wave-seeded")) return;
    sessionStorage.setItem("wave-seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify({ advanced: { protectLibrary: false }, view: extra }));
  }, view);
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await page.locator('[role="gridcell"][data-col="title"]').nth(3).dblclick();
  await expect(deck(page).getByRole("button", { name: "Play", exact: true })).toBeEnabled();
}

async function waveformChoices(page: Page) {
  await page.getByRole("banner").getByRole("button", { name: "Settings" }).click();
  const dialog = page.getByRole("dialog", { name: "Preferences" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("tab", { name: "Color" }).click();
  return { dialog, group: dialog.getByRole("radiogroup", { name: "Waveform color" }) };
}

test("the theme file's waveform theme wins over Preferences, which shows it greyed; without it the preference is back", async ({ page }) => {
  await openDeck(page, { waveformColor: "rgb" });
  const overview = deckOverview(page);
  await expect.poll(() => pixels(overview)).not.toBe("");
  const rgb = await pixels(overview);

  await showTheme(page, { waveform: "theme", wave: WAVE });
  await expect.poll(() => paints(overview, PINK)).toBe(true);
  let { dialog, group } = await waveformChoices(page);
  await expect(group.getByRole("radio", { name: "Theme", exact: true })).toBeChecked();
  for (const name of ["BLUE", "RGB", "3Band", "Theme"]) {
    await expect(group.getByRole("radio", { name, exact: true })).toBeDisabled();
  }
  await expect(dialog.getByText("Set by the theme file (waveform).")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();

  // The file no longer names one: the DJ's own choice, untouched, is back.
  await showTheme(page, { wave: WAVE });
  await expect.poll(() => pixels(overview)).toBe(rgb);
  ({ dialog, group } = await waveformChoices(page));
  await expect(group.getByRole("radio", { name: "RGB", exact: true })).toBeChecked();
  await expect(group.getByRole("radio", { name: "RGB", exact: true })).toBeEnabled();
  await expect(dialog.getByText("Set by the theme file (waveform).")).toHaveCount(0);
});

test("the Theme waveform takes the Omarchy palette's colours, never the window's roles", async ({ page }) => {
  await openDeck(page, { waveformColor: "theme" });
  const overview = deckOverview(page);
  // No Omarchy theme: Tokyo Night's blue.
  await expect.poll(() => paints(overview, [122, 162, 247])).toBe(true);

  await showTheme(page, { roles: roles("#00ff00"), wave: WAVE });
  await expect.poll(() => paints(overview, PINK)).toBe(true);
  const drawn = await pixels(overview);
  // Another chrome (a theme file's colours, say), the same Omarchy palette: the same waveform.
  await showTheme(page, { roles: roles("#ff00ff"), wave: WAVE });
  await page.waitForTimeout(200);
  expect(await pixels(overview)).toBe(drawn);
  // 3Band never moves with any of it.
  await showTheme(page, { roles: roles("#ff00ff"), wave: WAVE, waveform: "3band" });
  await expect.poll(() => paints(overview, MEASURED_BLUE)).toBe(true);
  expect(await paints(overview, PINK)).toBe(false);
});

test("the compact player's overview and Intake's large detail draw in the waveform theme in force", async ({ page }) => {
  await seedShell(page);
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  await page.locator('[role="gridcell"][data-col="title"]').nth(3).dblclick();
  const compact = shellPlayer(page).getByTestId("compact-player-overview").locator("canvas").first();
  await expect.poll(() => paints(compact, MEASURED_BLUE)).toBe(true);

  await showTheme(page, { waveform: "theme", wave: WAVE });
  await expect.poll(() => paints(compact, PINK)).toBe(true);
  expect(await paints(compact, MEASURED_BLUE)).toBe(false);

  await page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "Intake" }).click();
  const queued = page.getByRole("listbox", { name: "Intake queue" }).getByRole("option").filter({ hasText: /· \d{2,3} ·/ }).first();
  await queued.click();
  const large = shellPlayer(page).getByTestId("large-player-wave").locator("canvas").first();
  await expect.poll(() => paints(large, PINK)).toBe(true);
  await expect.poll(() => paints(shellPlayer(page).getByTestId("compact-player-overview").locator("canvas").first(), PINK)).toBe(true);

  await showTheme(page, { waveform: "blue" });
  await expect.poll(() => paints(large, PINK)).toBe(false);
});

const GREEN = [64, 160, 43] as const;
const GREEN_WAVE = { low: "#40a02b", mid: "#df8e1d", high: "#eff1f5" };

test("a row preview never keeps the last theme's pixels: recoloured from its bytes, or cleared when a read fails", async ({ page }) => {
  await openDeck(page, { waveformColor: "theme" });
  await showTheme(page, { wave: WAVE });
  const rows = page.locator("[data-preview-wave]");
  const firstRows = async () => {
    const n = Math.min(await rows.count(), 6);
    return Array.from({ length: n }, (_, i) => rows.nth(i));
  };
  // Drawn, in the Omarchy theme's pink.
  await expect.poll(async () => (await Promise.all((await firstRows()).map((r) => paints(r, PINK)))).filter(Boolean).length).toBeGreaterThan(2);

  // Every read fails from now on.
  await page.evaluate(() => (window as unknown as { __failWaveformReads: (on: boolean) => void }).__failWaveformReads(true));

  // Another Omarchy theme: the rows recolour from the bytes they already have.
  await showTheme(page, { wave: GREEN_WAVE });
  await expect.poll(async () => (await Promise.all((await firstRows()).map((r) => paints(r, PINK)))).some(Boolean)).toBe(false);
  await expect.poll(async () => (await Promise.all((await firstRows()).map((r) => paints(r, GREEN)))).filter(Boolean).length).toBeGreaterThan(2);

  // Another tag kind (RGB), whose bytes were never read, and the read fails:
  // the rows are cleared and say so, not left in green.
  await showTheme(page, { wave: GREEN_WAVE, waveform: "rgb" });
  await expect.poll(async () => (await Promise.all((await firstRows()).map((r) => paints(r, GREEN)))).some(Boolean)).toBe(false);
  await expect.poll(() => page.locator("[data-preview-wave][data-wave-failed]").count()).toBeGreaterThan(2);
});

/**
 * Each waveform canvas's own pixels against what the screen shows of it.
 *
 * Everything but the canvases is hidden and the page is captured at device
 * resolution; each canvas's store is then laid against the device pixels
 * under its box. A store of the right size that the engine still resamples
 * (a box a fraction of a device pixel off, or a capped ratio) blends every
 * one-pixel bar into its neighbour and fails here, not only a wrong size.
 */
async function sharpness(page: Page, selector: string, most = 4): Promise<{ count: number; blurred: string[] }> {
  const boxes = await page.evaluate(([sel, n]) => {
    const style = document.createElement("style");
    style.id = "only-canvases";
    style.textContent = "* { visibility: hidden !important; } canvas { visibility: visible !important; }";
    document.head.append(style);
    const canvases = [...document.querySelectorAll<HTMLCanvasElement>(sel)]
      .filter((c) => c.width > 0 && c.getBoundingClientRect().width > 0 && c.getBoundingClientRect().top >= 0
        && c.getBoundingClientRect().bottom <= innerHeight)
      .slice(0, n);
    canvases.forEach((c, i) => c.setAttribute("data-sharp", String(i)));
    return canvases.map((c) => {
      const r = c.getBoundingClientRect();
      return { x: r.left, y: r.top, w: r.width, h: r.height };
    });
  }, [selector, most] as const);
  const shot = await page.screenshot({ scale: "device", animations: "disabled" });
  const blurred = await page.evaluate(async ([png, sel]) => {
    document.getElementById("only-canvases")?.remove();
    const image = new Image();
    image.src = `data:image/png;base64,${png}`;
    await image.decode();
    const screen = document.createElement("canvas");
    screen.width = image.width;
    screen.height = image.height;
    const sctx = screen.getContext("2d", { willReadFrequently: true })!;
    sctx.drawImage(image, 0, 0);
    const ratio = image.width / innerWidth;
    const out: string[] = [];
    for (const c of document.querySelectorAll<HTMLCanvasElement>("[data-sharp]")) {
      c.removeAttribute("data-sharp");
      const r = c.getBoundingClientRect();
      // Only what is on screen: the viewport and every ancestor that clips
      // (the detail is a strip twice the window's width, slid under its well).
      let [vl, vt, vr, vb] = [0, 0, innerWidth, innerHeight];
      for (let el = c.parentElement; el; el = el.parentElement) {
        if (getComputedStyle(el).overflow === "visible") continue;
        const e = el.getBoundingClientRect();
        [vl, vt, vr, vb] = [Math.max(vl, e.left), Math.max(vt, e.top), Math.min(vr, e.right), Math.min(vb, e.bottom)];
      }
      const sx = Math.max(0, Math.ceil((Math.max(vl, r.left) - r.left) * ratio));
      const sy = Math.max(0, Math.ceil((Math.max(vt, r.top) - r.top) * ratio));
      const sw = Math.min(c.width, Math.floor((Math.min(vr, r.right) - r.left) * ratio)) - sx;
      const sh = Math.min(c.height, Math.floor((Math.min(vb, r.bottom) - r.top) * ratio)) - sy;
      if (sw <= 0 || sh <= 0) continue;
      const own = c.getContext("2d")!.getImageData(sx, sy, sw, sh).data;
      // Best of the reported place and a device pixel either way: Chromium
      // puts a composited layer (the detail's sliding strip) a device pixel
      // from where its box reports, and that is a shift, not a resampling. A
      // resampled store matches at no whole-pixel offset.
      let share = 0;
      let opaque = 0;
      for (const [dx, dy] of [[0, 0], [1, 0], [-1, 0], [0, 1], [0, -1]] as const) {
        const shown = sctx.getImageData(Math.round(r.left * ratio) + sx + dx, Math.round(r.top * ratio) + sy + dy, sw, sh).data;
        let seen = 0;
        let same = 0;
        for (let i = 0; i < own.length; i += 4) {
          if (own[i + 3] !== 255) continue;
          seen += 1;
          const near = (k: number) => Math.abs((own[i + k] ?? 0) - (shown[i + k] ?? 0)) <= 2;
          if (near(0) && near(1) && near(2)) same += 1;
        }
        opaque = seen;
        share = Math.max(share, seen === 0 ? 1 : same / seen);
        if (share >= 0.995) break;
      }
      const sized = Math.abs(r.width * ratio - c.width) < 0.1 && Math.abs(r.height * ratio - c.height) < 0.1;
      if (share < 0.995 || !sized) {
        out.push(`${sel}: store ${c.width}x${c.height} over ${(r.width * ratio).toFixed(2)}x${(r.height * ratio).toFixed(2)} at ${(r.left * ratio).toFixed(2)},${(r.top * ratio).toFixed(2)}, ${(share * 100).toFixed(1)}% of ${opaque} pixels as drawn`);
      }
    }
    return out;
  }, [shot.toString("base64"), selector] as const);
  return { count: boxes.length, blurred };
}

const DECK = [
  ["overview", '[data-testid="player-overview"] canvas'],
  ["detail", '[data-testid="player-detail"] canvas'],
  ["row previews", '[role="gridcell"] canvas'],
] as const;
const SHELL = [
  ["compact overview", '[data-testid="compact-player-overview"] canvas'],
  ["large detail", '[data-testid="large-player-wave"] canvas'],
] as const;

async function expectSharp(page: Page, which: readonly (readonly [string, string])[], label: string) {
  for (const [where, selector] of which) {
    await expect.poll(async () => (await sharpness(page, selector, 1)).count, `${label} ${where} drawn`).toBeGreaterThan(0);
    // Until the waveform has settled (it grows while its bytes arrive): a
    // store the engine resamples never settles to its own pixels.
    await expect.poll(async () => (await sharpness(page, selector)).blurred, { message: `${label} ${where}`, timeout: 8000 }).toEqual([]);
  }
}

async function openIntakeLarge(page: Page, step: number) {
  await page.evaluate((s) => {
    localStorage.setItem("rbl.preferences", JSON.stringify({ advanced: { protectLibrary: false }, view: { viewShell: true, uiScale: s } }));
    sessionStorage.setItem("wave-seeded", "1");
  }, step);
  await page.reload();
  await expect(page.getByTestId("shell")).toBeVisible();
  await page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "Intake" }).click();
  await page.getByRole("listbox", { name: "Intake queue" }).getByRole("option").filter({ hasText: /· \d{2,3} ·/ }).first().click();
}

/**
 * Each canvas's store exactly the device pixels its box covers, to layout's
 * precision (a 64th of a CSS pixel).
 *
 * The pixels themselves are compared (`sharpness`) only where Chromium can
 * place a canvas on whole device pixels: an integer display ratio with no
 * interface scale. Under a browser tab's stand-in for the scale (CSS `zoom`)
 * Chromium resamples a canvas whatever its geometry (74 to 90% of an
 * overview's pixels as drawn, at whole-pixel positions), and at a fractional
 * display ratio a box lands a fraction in that no layout offset can remove
 * (offsets are 64ths of a CSS pixel) and that Chromium resamples even at a
 * hundredth. WebKitGTK, the app's engine, places canvases on device pixels
 * itself, and on Linux never runs at a fractional display ratio (GTK draws at
 * 2x and the compositor scales the window); its pixels are compared at every
 * interface scale in the real app (`headless-driver/measure.py`).
 */
const geometry = (page: Page, selector: string) => page.evaluate((sel) => {
  const out: string[] = [];
  const canvases = [...document.querySelectorAll<HTMLCanvasElement>(sel)].filter((c) => c.width > 0 && c.getBoundingClientRect().width > 0);
  const ratio = devicePixelRatio;

  for (const c of canvases.slice(0, 6)) {
    const r = c.getBoundingClientRect();
    // A tenth of a device pixel: layout keeps lengths in 64ths of a CSS pixel.
    if (Math.abs(r.width * ratio - c.width) > 0.1 || Math.abs(r.height * ratio - c.height) > 0.1) {
      out.push(`${sel}: store ${c.width}x${c.height} over ${(r.width * ratio).toFixed(3)}x${(r.height * ratio).toFixed(3)}`);
    }
  }
  return { count: canvases.length, out };
}, selector);

async function expectExact(page: Page, which: readonly (readonly [string, string])[], label: string) {
  for (const [where, selector] of which) {
    await expect.poll(async () => (await geometry(page, selector)).count, `${label} ${where} drawn`).toBeGreaterThan(0);
    await expect.poll(async () => (await geometry(page, selector)).out, { message: `${label} ${where}`, timeout: 5000 }).toEqual([]);
  }
}

// 3 is past the old 2x cap.
for (const ratio of [1, 2, 3]) {
  test.describe(`at a ${ratio}x display`, () => {
    test.use({ deviceScaleFactor: ratio });
    test("every waveform's pixels on screen are its own", async ({ page }) => {
      await openDeck(page, { uiScale: 4 });
      await expectSharp(page, DECK, `${ratio}x`);
      await openIntakeLarge(page, 4);
      await expectSharp(page, SHELL, `${ratio}x`);
    });
  });
}

// 1.5 and 2.5 are fractional display ratios, and past the old 2x cap.
for (const ratio of [1, 1.5, 2, 2.5, 3]) {
  test.describe(`at a ${ratio}x display, scaled`, () => {
    test.use({ deviceScaleFactor: ratio });
    // The smallest and the largest interface scale, and a fractional one between.
    for (const step of [0, 4, 6, 7]) {
      test(`every waveform's store is the device pixels it covers at scale step ${step}`, async ({ page }) => {
        await openDeck(page, { uiScale: step });
        await expectExact(page, DECK, `step ${step}`);
        await openIntakeLarge(page, step);
        await expectExact(page, SHELL, `step ${step}`);
      });
    }
  });
}

test.describe("live changes", () => {
  test.use({ deviceScaleFactor: 2 });

  test("the scale changed with the keys redraws every waveform exact, step by step, and back to its own pixels", async ({ page }) => {
    await openDeck(page, { uiScale: 4 });
    const zoom = () => page.evaluate(() => document.documentElement.style.getPropertyValue("zoom") || "1");
    for (const [key, expected] of [["ControlOrMeta+=", "1.1"], ["ControlOrMeta+=", "1.25"], ["ControlOrMeta+-", "1.1"], ["ControlOrMeta+-", "1"], ["ControlOrMeta+-", "0.9"], ["ControlOrMeta+=", "1"]] as const) {
      await page.keyboard.press(key);
      await expect.poll(zoom).toBe(expected);
      await expectExact(page, DECK, `zoom ${expected}`);
      if (expected === "1") await expectSharp(page, DECK, `zoom ${expected}`);
    }
  });

  test("a paused waveform is redrawn at the new density when the display's ratio changes with no resize", async ({ page }) => {
    // A display move tells the page through its `resolution` media queries;
    // DevTools' emulation changes the ratio but fires nothing, so the page's
    // own queries are kept and fired as the move would fire them.
    await page.addInitScript(() => {
      const made: MediaQueryList[] = [];
      const real = window.matchMedia.bind(window);
      window.matchMedia = (query: string) => {
        const list = real(query);
        if (query.includes("resolution")) made.push(list);
        return list;
      };
      (window as unknown as { __displayMoved: () => void }).__displayMoved = () => {
        for (const list of made.splice(0)) list.dispatchEvent(new Event("change"));
      };
    });
    await openDeck(page, { uiScale: 4 });
    const cdp = await page.context().newCDPSession(page);
    const move = async (deviceScaleFactor: number) => {
      await cdp.send("Emulation.setDeviceMetricsOverride", { ...viewport, deviceScaleFactor, mobile: false });
      await expect.poll(() => page.evaluate(() => devicePixelRatio)).toBe(deviceScaleFactor);
      await page.evaluate(() => (window as unknown as { __displayMoved: () => void }).__displayMoved());
    };
    const viewport = page.viewportSize()!;
    await expectSharp(page, DECK.slice(0, 2), "2x");
    const detailWidth = () => page.locator('[data-testid="player-detail"] canvas').first().evaluate((c: HTMLCanvasElement) => c.width);
    const before = await detailWidth();
    // Moved to a 3x display: same window, same CSS size, nothing playing.
    await move(3);
    await expect.poll(detailWidth).toBe(Math.round(before * 1.5));
    await expectSharp(page, DECK, "3x");
    // And to a fractional 2.5x one.
    await move(2.5);
    await expect.poll(detailWidth).toBe(Math.round(before * 1.25));
    await expectExact(page, DECK, "2.5x");

    // The shell's player canvases too.
    await openIntakeLarge(page, 4);
    await expectExact(page, SHELL, "2.5x shell");
    await move(1);
    await expectSharp(page, SHELL, "1x shell");
  });
});
