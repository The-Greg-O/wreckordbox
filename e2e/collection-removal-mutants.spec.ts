import { expect, test, type Page } from "@playwright/test";

/**
 * Remove from Collection names the Mutants that would go with the tracks
 * before the DJ confirms (wbx-80). The mock has no Sidecar: the test lists
 * the Mutants the service would report in `window.__mockMutants` (a
 * `trackId` of "*" stands for any track asked about), or a reason in
 * `window.__mockMutantsFail`. The mock answers every confirmation with OK
 * unless `window.__confirmAnswer = false`, and keeps what it was asked in
 * `window.__confirmed`.
 */

type Page$ = {
  __confirmAnswer?: boolean;
  __confirmed?: string[];
  __mockMutants?: unknown[];
  __mockMutantsFail?: string;
};

const rows = (page: Page) => page.getByRole("row").filter({ has: page.getByRole("gridcell") });
const selected = (page: Page) => rows(page).and(page.locator("[data-selected]"));
const status = (page: Page) => page.getByRole("contentinfo");
const asked = (page: Page) => page.evaluate(() => (window as unknown as Page$).__confirmed ?? []);

async function openCollection(page: Page) {
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await page.locator('[role="treeitem"][data-kind="collection"]').click();
  await rows(page).nth(2).click();
  await expect(selected(page)).toHaveCount(1);
}

async function removeSelected(page: Page) {
  await rows(page).nth(2).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Remove from Collection" }).click();
}

test("the confirmation names every Mutant that would go, and Cancel removes nothing", async ({ page }) => {
  await openCollection(page);
  await page.evaluate(() => {
    const w = window as unknown as Page$;
    w.__confirmAnswer = false;
    w.__mockMutants = [
      { id: 3, playlistId: "1", playlistName: "Friday", position: 2, trackId: "*", title: "Night Bus", description: "+1 st" },
      { id: 7, playlistId: "2", playlistName: "Saturday", position: 5, trackId: "*", title: "Night Bus", description: "no vocals" },
    ];
  });
  const before = await rows(page).count();
  await removeSelected(page);
  await expect.poll(() => asked(page)).toHaveLength(1);
  const [question] = await asked(page);
  expect(question).toBe(
    "Remove 1 track from the collection? This can’t be undone. The files stay where they are.\n\n" +
      "These 2 Mutants will be removed with them:\n" +
      "• Mutant 3: “Night Bus” (entry 2 of Friday), +1 st\n" +
      "• Mutant 7: “Night Bus” (entry 5 of Saturday), no vocals",
  );
  await page.waitForTimeout(500);
  await expect(rows(page)).toHaveCount(before);
  await expect(status(page).filter({ hasText: "Removed" })).toHaveCount(0);
});

test("a track with no Mutant is asked about as before, and OK removes it", async ({ page }) => {
  await openCollection(page);
  await removeSelected(page);
  await expect.poll(() => asked(page)).toEqual([
    "Remove 1 track from the collection? This can’t be undone. The files stay where they are.",
  ]);
  await expect(status(page)).toContainText("Removed 1 track from the collection.");
});

test("Mutants that cannot be read are said in the question, not hidden", async ({ page }) => {
  await openCollection(page);
  await page.evaluate(() => {
    const w = window as unknown as Page$;
    w.__confirmAnswer = false;
    w.__mockMutantsFail = "the Sidecar is busy";
  });
  await removeSelected(page);
  await expect.poll(() => asked(page)).toEqual([
    "Remove 1 track from the collection? This can’t be undone. The files stay where they are.\n\n" +
      "The Mutants of these tracks could not be checked: the Sidecar is busy",
  ]);
});
