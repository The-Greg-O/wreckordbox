/**
 * The title bar's readout shows the device's own underruns (wbx-45.20): the
 * audio that was actually lost, heard as a click or a gap, beside the late
 * callbacks the AUDIO figure already summed. The mock backend reads the
 * engine's health from `window.__mockAudioHealth`; the readout polls, so a
 * burst between two readings arrives as one new count.
 */
import { expect, test, type Page } from "@playwright/test";

type Health = { underruns: number; latencyMs: number | null };

const setHealth = (page: Page, health: Health) =>
  page.evaluate((next) => {
    (window as unknown as { __mockAudioHealth?: Health }).__mockAudioHealth = next;
  }, health);

test("device underruns reach the readout and are marked once there are any", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __mockAudioHealth?: Health }).__mockAudioHealth = { underruns: 0, latencyMs: null };
  });
  await page.goto("/");
  const underruns = page.getByTestId("app-cost-underruns");
  await expect(underruns).toHaveText("UNDERRUNS 0");
  await expect(underruns).not.toHaveAttribute("data-lost", "true");
  await expect(underruns).toHaveAttribute("aria-label", "Device underruns: 0; output latency unknown");

  // A burst of three, then one more, before the next reading: one count of four.
  await setHealth(page, { underruns: 3, latencyMs: 197.4 });
  await setHealth(page, { underruns: 4, latencyMs: 197.4 });
  await expect(underruns).toHaveText("UNDERRUNS 4", { timeout: 15_000 });
  await expect(underruns).toHaveAttribute("data-lost", "true");
  await expect(underruns).toHaveAttribute("aria-label", "Device underruns: 4; output latency 197 ms");
});
