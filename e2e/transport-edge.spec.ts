/**
 * Preferences › Audio › Transport edge (wbx-45.16): how play, pause, cue and
 * beat jumps meet the music. Immediate by default (the DJ's decision), or a 1
 * or 2 ms fade, for the DJ to compare by ear. Checked where it lands: the
 * engine call (`window.__transportEdge` in the mock), live while a deck
 * plays, kept across a restart, and in both the old layout and the shell's
 * compact player, which share the one engine.
 */
import { expect, test, type Page } from "@playwright/test";

import { load as loadCompact, pauseButton, playButton } from "./compactPlayer";

const engineEdge = (page: Page) =>
  page.evaluate(() => (window as unknown as { __transportEdge?: string }).__transportEdge ?? null);

async function open(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
}

async function audioPrefs(page: Page) {
  await page.getByRole("banner").getByRole("button", { name: "Settings" }).click();
  const dialog = page.getByRole("dialog", { name: "Preferences" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("tab", { name: "Audio" }).click();
  return dialog;
}

const edges = (dialog: ReturnType<Page["getByRole"]>) => dialog.getByRole("radiogroup", { name: "Transport edge" });
const oldPlayer = (page: Page) => page.getByRole("region", { name: "Preview player" });

test("the transport edge starts immediate, changes live while a deck plays, and is kept", async ({ page }) => {
  await open(page);
  await expect.poll(() => engineEdge(page)).toBe("immediate");

  // A deck playing in the old layout, so the change is made under it.
  await page.locator('[role="gridcell"][data-col="title"]').nth(3).dblclick();
  const play = oldPlayer(page).getByRole("button", { name: "Play", exact: true });
  await expect(play).toBeEnabled();
  await play.click();
  const pause = oldPlayer(page).getByRole("button", { name: "Pause", exact: true });
  await expect(pause).toBeVisible();

  const dialog = await audioPrefs(page);
  await expect(dialog.getByRole("heading", { name: "Transport edge" })).toBeVisible();
  const group = edges(dialog);
  await expect(group.getByRole("radio")).toHaveCount(3);
  await expect(group.getByRole("radio", { name: "Immediate" })).toBeChecked();
  await group.getByRole("radio", { name: "2 ms" }).check();
  await expect.poll(() => engineEdge(page)).toBe("2ms");
  // By keyboard too: the arrow moves the choice, and the engine follows.
  await group.getByRole("radio", { name: "2 ms" }).focus();
  await page.keyboard.press("ArrowUp");
  await expect(group.getByRole("radio", { name: "1 ms" })).toBeChecked();
  await expect.poll(() => engineEdge(page)).toBe("1ms");
  // Live: the deck played on through both changes.
  await expect(pause).toBeAttached();

  // Kept: a restart reads it back and hands it to the engine before anything plays.
  await page.reload();
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await expect.poll(() => engineEdge(page)).toBe("1ms");
  const again = await audioPrefs(page);
  await expect(edges(again).getByRole("radio", { name: "1 ms" })).toBeChecked();
});

test("the compact player's engine gets the stored edge, and plays with it", async ({ page }) => {
  await loadCompact(page, "", 3, {});
  await page.evaluate(() => {
    const stored = JSON.parse(localStorage.getItem("rbl.preferences") ?? "{}");
    localStorage.setItem("rbl.preferences", JSON.stringify({ ...stored, audio: { ...stored.audio, transportEdge: "2ms" } }));
  });
  await page.reload();
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await expect.poll(() => engineEdge(page)).toBe("2ms");
  await page.locator('[role="gridcell"][data-col="title"]').nth(3).dblclick();
  await expect(playButton(page)).toBeEnabled();
  await playButton(page).click();
  await expect(pauseButton(page)).toBeVisible();
  await expect.poll(() => engineEdge(page)).toBe("2ms");
});
