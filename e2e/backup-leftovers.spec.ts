/**
 * The unfinished copies a refused backup left on a Stick (wbx-45.25): offered
 * in Backups, removed only after the Stick is unlocked and the removal held
 * for, and only those names. The mock's safety log refuses the removal on a
 * protected Stick or without the hold's token.
 */
import { expect, test, type Page } from "@playwright/test";

import { confirmStickWrite, enableViewShell } from "./helpers";

const TEST = "/Volumes/TEST";
const LEFT = [".partial-3f0c6a52-7d1e-4b8a-9c2f-0a1b2c3d4e5f", ".partial-3f0c6a52-7d1e-4b8a-9c2f-0a1b2c3d4e5f.zip"];

type Hooks = {
  __backupLeftovers?: { stickRoot: string; names: string[] };
  __stickWrites: () => { method: string; path: string }[];
  __stickRefusedWrites: () => { method: string; path: string }[];
};

async function openBackups(page: Page) {
  await page.getByRole("button", { name: "Command palette", exact: true }).click();
  await page.getByRole("combobox", { name: "Search actions" }).fill("preferences");
  await page.keyboard.press("Enter");
  const preferences = page.getByRole("dialog", { name: "Preferences", exact: true });
  await preferences.getByRole("tab", { name: "Backups", exact: true }).click();
  return preferences;
}

test("a refused backup's unfinished copies are removed only after the Stick is unlocked and the removal held for", async ({ page }) => {
  await enableViewShell(page);
  await page.addInitScript(([root, names]) => {
    (window as unknown as Hooks).__backupLeftovers = { stickRoot: root, names: [...names] };
  }, [TEST, LEFT] as const);
  await page.goto("/?writable=1");
  await expect(page.getByRole("group", { name: "Stick TEST", exact: true })).toBeVisible();

  const preferences = await openBackups(page);
  const offer = preferences.getByRole("region", { name: "Unfinished backups" });
  await expect(offer.getByRole("list", { name: "Unfinished backup copies" }).getByRole("listitem")).toHaveText(LEFT);
  await expect(offer).toContainText("TEST is protected: unlock it to remove them.");

  // Cancelled at the dialog: nothing removed, still offered.
  await offer.getByRole("button", { name: "Remove unfinished copies…" }).click();
  const dialog = page.getByRole("alertdialog", { name: /^Write to / });
  await expect(dialog).toContainText(`Remove “${LEFT[0]}” (unfinished backup)`);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as Hooks).__stickWrites().filter(w => w.method === "sweepBackupLeftovers"))).toEqual([]);
  await expect(offer.getByRole("listitem")).toHaveCount(2);

  // Unlocked in the dialog, held for: exactly the recorded copies go.
  await offer.getByRole("button", { name: "Remove unfinished copies…" }).click();
  await confirmStickWrite(page);
  await expect(offer.getByRole("status")).toHaveText("Removed 2 unfinished backup copies from TEST.");
  await expect(offer.getByRole("listitem")).toHaveCount(0);
  const writes = await page.evaluate(() => (window as unknown as Hooks).__stickWrites().filter(w => w.method === "sweepBackupLeftovers"));
  expect(writes).toEqual([expect.objectContaining({ path: TEST, args: { names: LEFT } })]);
});
