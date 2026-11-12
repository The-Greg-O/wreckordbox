import { expect, test, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";

const stopped = "The Library service has stopped, so changes cannot be saved. Restart wreckordbox to carry on editing.";
const line = (page: Page) => page.getByRole("status", { name: "Library service" });

async function setRuns(page: Page, runs: { id: number; description: string }[]) {
  await page.evaluate((runs) => {
    (window as unknown as { __setActiveRuns: (runs: { id: number; description: string }[]) => void }).__setActiveRuns(runs);
  }, runs);
}

test.beforeEach(async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
});

test("connected service, a Run starting and finishing, and view switches", async ({ page }) => {
  await expect(line(page)).toContainText("Service connected");
  await setRuns(page, [{ id: 1, description: "Tag the backlog" }]);
  await expect(line(page)).toContainText("Tag the backlog");
  for (const name of ["Intake", "Review", "Stick", "Library"]) {
    await page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name }).click();
    await expect(line(page)).toContainText("Tag the backlog");
  }
  await setRuns(page, []);
  await expect(line(page)).not.toContainText("Tag the backlog");
  await expect(line(page)).toContainText("Service connected");
});

test("concurrent Runs become a count, then the remaining description", async ({ page }) => {
  await setRuns(page, [{ id: 1, description: "Tag the backlog" }, { id: 2, description: "Measure Intensity" }]);
  await expect(line(page)).toContainText("2 Runs in progress");
  await setRuns(page, [{ id: 2, description: "Measure Intensity" }]);
  await expect(line(page)).toContainText("Measure Intensity");
  await expect(line(page)).not.toContainText("2 Runs");
  await setRuns(page, []);
  await expect(line(page)).not.toContainText("Measure Intensity");
});

test("stopped service stays visible through view switches and later refreshes", async ({ page }) => {
  await expect(line(page)).toContainText("Service connected");
  await setRuns(page, [{ id: 1, description: "Tag the backlog" }]);
  await expect(line(page)).toContainText("Tag the backlog");
  await page.evaluate((message) => {
    (window as unknown as { __stopService: (message: string) => void }).__stopService(message);
  }, stopped);
  await expect(line(page)).toContainText(stopped);
  await expect(line(page)).not.toContainText("Service connected");
  await expect(line(page)).not.toContainText("Tag the backlog");
  await page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "Review" }).click();
  await page.waitForTimeout(1500);
  await expect(line(page)).toContainText(stopped);
});

test("long descriptions fit a quiet 22px line at narrow widths", async ({ page }) => {
  await page.setViewportSize({ width: 436, height: 800 });
  const description = "Measure Intensity for the tracks waiting for Intake ".repeat(4);
  await setRuns(page, [{ id: 1, description }]);
  await expect(line(page)).toContainText(description.trim());
  const box = await line(page).boundingBox();
  expect(box?.height).toBe(22);
  expect(box!.x + box!.width).toBeLessThanOrEqual(436);
  expect(await line(page).evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("work and a stopped-service message are read even if they predate the window", async ({ page }) => {
  await page.addInitScript(() => {
    (window as unknown as { __initialActiveRuns: { id: number; description: string }[] }).__initialActiveRuns = [
      { id: 7, description: "A Run already in progress" },
    ];
  });
  await page.reload();
  await expect(line(page)).toContainText("A Run already in progress");
  await page.addInitScript((message) => {
    (window as unknown as { __initialServiceProblem: string }).__initialServiceProblem = message;
  }, stopped);
  await page.reload();
  await expect(line(page)).toContainText(stopped);
  await expect(line(page)).not.toContainText("A Run already in progress");
  await page.setViewportSize({ width: 436, height: 800 });
  expect(await line(page).evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
});
