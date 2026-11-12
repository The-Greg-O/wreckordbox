import { expect, test } from "@playwright/test";

test("analysis completes one operation and shows a Ledger failure separately from its results", async ({ page }) => {
  await page.goto("/?writable=1&ledgerWarning=1");
  const rows = page.getByRole("row").filter({ has: page.getByRole("gridcell") });
  await rows.nth(2).click();
  await rows.nth(3).click({ modifiers: ["Shift"] });
  await page.keyboard.press("Shift+Meta+A");
  const dialog = page.getByRole("dialog", { name: "Analysis Setting" });
  await dialog.getByRole("button", { name: "OK", exact: true }).click();
  await expect(page.getByRole("contentinfo")).toContainText("Library changes were accepted, but their Ledger recording is incomplete.");
  const runs = await page.evaluate(() => (window as unknown as {
    __bulkLedgerRuns: { kind: string; tracks: string[]; ended: boolean }[];
  }).__bulkLedgerRuns);
  expect(runs).toHaveLength(1);
  expect(runs[0]).toMatchObject({ kind: "analysis", ended: true });
  expect(new Set(runs[0]!.tracks).size).toBe(2);
  await expect(page.getByRole("contentinfo").getByRole("button", { name: "Stop" })).toHaveCount(0);
});
