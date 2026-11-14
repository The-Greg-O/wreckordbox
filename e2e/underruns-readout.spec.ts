/**
 * The window shows the device's own underruns (wbx-45.20): the audio that
 * was actually lost, heard as a click or a gap, beside the late callbacks the
 * audio meter already summed. On Linux the title bar (and the macOS readout
 * in it) is not drawn, so the count sits in the old layout's top bar and the shell's status line. The
 * mock backend reads the engine's health from `window.__mockAudioHealth`; the
 * readout polls, so a burst between two readings arrives as one new count.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";

import { enableViewShell } from "./helpers";

type Health = { underruns: number; latencyMs: number | null };

const setHealth = (page: Page, health: Health) =>
  page.evaluate((next) => {
    (window as unknown as { __mockAudioHealth?: Health }).__mockAudioHealth = next;
  }, health);

async function counts(page: Page, where: Locator) {
  const underruns = where.getByRole("status", { name: /^Device underruns/ });
  await expect(underruns).toBeVisible();
  await expect(underruns).toHaveText("UNDERRUNS 0", { useInnerText: true });
  await expect(underruns).not.toHaveAttribute("data-lost", "true");
  await expect(underruns).toHaveAccessibleName("Device underruns: 0; output latency unknown");

  // A burst of three, then one more, before the next reading: one count of four.
  await setHealth(page, { underruns: 3, latencyMs: 197.4 });
  await setHealth(page, { underruns: 4, latencyMs: 197.4 });
  await expect(underruns).toHaveText("UNDERRUNS 4", { timeout: 15_000, useInnerText: true });
  await expect(underruns).toBeVisible();
  await expect(underruns).toHaveAttribute("data-lost", "true");
  await expect(underruns).toHaveAccessibleName("Device underruns: 4; output latency 197 ms");
  // The error colour, not the readout's dim grey.
  const colour = await underruns.evaluate((node) => getComputedStyle(node).color);
  const dim = await page.evaluate(() => {
    const probe = document.createElement("span");
    probe.style.color = "var(--c-text-dim)";
    document.body.append(probe);
    const value = getComputedStyle(probe).color;
    probe.remove();
    return value;
  });
  expect(colour).not.toBe(dim);
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __mockAudioHealth?: Health }).__mockAudioHealth = { underruns: 0, latencyMs: null };
  });
});

test("the old layout's top bar counts device underruns", async ({ page }) => {
  await page.goto("/");
  await counts(page, page.getByRole("banner"));
});

test("the view shell's status line counts device underruns", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  await counts(page, page.getByTestId("shell-status"));
});

test("at half width the shell hides a zero and still shows a count that matters, inside the status line", async ({ page }) => {
  await enableViewShell(page);
  await page.setViewportSize({ width: 640, height: 900 });
  await page.goto("/");
  await expect(page.getByTestId("shell")).toBeVisible();
  const underruns = page.getByTestId("shell-status").getByTestId("underrun-count");
  await expect(underruns).toHaveAttribute("aria-label", "Device underruns: 0; output latency unknown");
  await expect(underruns).toBeHidden();

  await setHealth(page, { underruns: 2, latencyMs: 197.4 });
  await expect(underruns).toBeVisible({ timeout: 15_000 });
  await expect(underruns).toHaveAccessibleName("Device underruns: 2; output latency 197 ms");
  await expect(underruns).toHaveText("⚠ 2", { useInnerText: true });
  const line = page.getByTestId("shell-status");
  for (const control of [underruns, ...(await line.locator("button").all())]) {
    const box = (await control.boundingBox())!;
    expect(box.x + box.width).toBeLessThanOrEqual(640.5);
  }
});
