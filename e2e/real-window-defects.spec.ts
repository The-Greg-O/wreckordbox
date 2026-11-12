/**
 * Defects the second integration pass found in the real WebKitGTK window
 * (wbx-45.15), each pinned here where a mock-backed browser can show it.
 */
import { expect, test, type Page } from "@playwright/test";
import { overview, player, titles } from "./compactPlayer";

/** The rightmost overview column with anything drawn in it, as a fraction; 0 with no waveform. */
function overviewDrawnTo(page: Page): Promise<number> {
  return page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>('[data-testid="shell-player"] [data-testid="compact-player-overview"] canvas');
    const context = canvas?.getContext("2d");
    if (!canvas || !context || canvas.width === 0 || canvas.height === 0) return 0;
    const { data, width, height } = context.getImageData(0, 0, canvas.width, canvas.height);
    for (let x = width - 1; x >= 0; x--) {
      for (let y = 0; y < height; y++) {
        if ((data[(y * width + x) * 4 + 3] ?? 0) > 0) return (x + 1) / width;
      }
    }
    return 0;
  });
}

const turnProtection = (page: Page, on: boolean) => page.evaluate(
  (value) => (window as unknown as { __turnLibraryProtection: (on: boolean) => void }).__turnLibraryProtection(value), on);

/** Opens the view shell on a writable mock Library with Library Protection on. */
async function openProtected(page: Page): Promise<void> {
  await page.addInitScript(() => {
    if (sessionStorage.getItem("defects-seeded")) return;
    sessionStorage.setItem("defects-seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify({ advanced: { protectLibrary: true }, view: { viewShell: true } }));
  });
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
}

/** The rows on screen with no tempo: unanalysed tracks. */
async function unanalysedRows(page: Page): Promise<number[]> {
  const bpms = page.locator('[role="gridcell"][data-col="bpm"]');
  const found = await bpms.evaluateAll((cells) => cells.flatMap((cell, i) => ((cell.textContent ?? "").trim() === "" ? [i] : [])));
  expect(found.length, "the mock list has unanalysed rows on screen").toBeGreaterThan(1);
  return found;
}

test("a track loaded before it was analysed shows its waveform once the analysis finishes, with no reload", async ({ page }) => {
  await openProtected(page);
  const bpm = (at: number) => page.locator('[role="gridcell"][data-col="bpm"]').nth(at);
  const dialog = page.getByRole("dialog", { name: "Analysis Setting" });
  let protectedNow = true;
  // The mock fails every seventh track's analysis on purpose: go on to the
  // next unanalysed row until one is analysed.
  let analysed = -1;
  for (const at of await unanalysedRows(page)) {
    // Loaded while the Library is protected: nothing analyses it on load.
    if (!protectedNow) await turnProtection(page, (protectedNow = true));
    const title = await titles(page).nth(at).innerText();
    await titles(page).nth(at).dblclick();
    await expect(player(page).getByTestId("compact-player-title")).toHaveText(title);
    await expect(overview(page)).toBeVisible();
    await page.waitForTimeout(300);
    expect(await overviewDrawnTo(page), "nothing to draw before the analysis").toBe(0);

    // Protection off, then the row analysed from the list as the DJ would.
    await turnProtection(page, (protectedNow = false));
    await titles(page).nth(at).click();
    await page.keyboard.press("Control+Shift+A");
    await dialog.getByRole("button", { name: "OK", exact: true }).click();
    await expect(page.getByTestId("shell-status")).not.toContainText("Analyzing", { timeout: 15_000 });
    if ((await bpm(at).innerText()).trim() !== "") {
      analysed = at;
      break;
    }
  }
  expect(analysed, "one track was analysed").toBeGreaterThanOrEqual(0);

  // The deck draws it, without the track being loaded again.
  await expect.poll(() => overviewDrawnTo(page), { timeout: 5_000 }).toBeGreaterThan(0.9);
});

test("a track that left the deck while it was being analysed shows its whole waveform when loaded again", async ({ page }) => {
  await page.addInitScript(() => {
    if (sessionStorage.getItem("defects-seeded")) return;
    sessionStorage.setItem("defects-seeded", "1");
    localStorage.setItem("rbl.preferences", JSON.stringify({ advanced: { protectLibrary: false }, view: { viewShell: true } }));
  });
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  const bpm = (at: number) => page.locator('[role="gridcell"][data-col="bpm"]').nth(at);
  const loadRow = async (at: number) => {
    const title = await titles(page).nth(at).innerText();
    await titles(page).nth(at).dblclick();
    await expect(player(page).getByTestId("compact-player-title")).toHaveText(title);
  };
  const bare = await unanalysedRows(page);
  const analysedRow = await page.locator('[role="gridcell"][data-col="bpm"]')
    .evaluateAll((cells) => cells.findIndex((cell) => (cell.textContent ?? "").trim() !== ""));

  // Loading an unanalysed track analyses it and draws it on the deck as it
  // decodes. The mock fails every seventh track before drawing: skip those.
  let drawing = -1;
  for (const at of bare) {
    await loadRow(at);
    const started = Date.now();
    while (Date.now() - started < 1_000 && await overviewDrawnTo(page) === 0) await page.waitForTimeout(25);
    const reached = await overviewDrawnTo(page);
    if (reached > 0 && reached < 0.8) {
      drawing = at;
      break;
    }
  }
  expect(drawing, "a loaded track started drawing").toBeGreaterThanOrEqual(0);

  // Part drawn, another track takes the deck; the analysis finishes meanwhile.
  await loadRow(analysedRow);
  await expect(bpm(drawing)).not.toHaveText("", { timeout: 15_000 });

  // Back on the deck, it is drawn whole, not as far as it had got.
  await loadRow(drawing);
  await expect.poll(() => overviewDrawnTo(page), { timeout: 5_000 }).toBeGreaterThan(0.9);
});
