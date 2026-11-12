import type { Page } from "@playwright/test";

/**
 * Tooltips default off; some assertions need the real text a hover shows.
 * Must run before the page navigates — it seeds the preference the same way
 * `grid-edit.spec.ts` does, rather than clicking through Settings each time.
 */
export async function enableTooltips(page: Page): Promise<void> {
  await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({ view: { tooltips: true } })));
}

/**
 * Turns the view shell on (ticket #50): the four-view frame drawn in place of
 * upstream's layout. Must run before the page navigates. It keeps the Library
 * writable, as the suite's own stored preferences do.
 */
export async function enableViewShell(page: Page): Promise<void> {
  await page.addInitScript(() => localStorage.setItem("rbl.preferences", JSON.stringify({
    advanced: { protectLibrary: false },
    view: { viewShell: true },
  })));
}

/**
 * Confirms the staged Stick write the window is asking about (wbx-45.4):
 * every write to a Stick shows its plan and starts only after the button
 * is held (two seconds for a Gig stick). Resolves once the dialog is gone.
 */
/**
 * Stick Protection (wbx-45.11): a protected Stick is unlocked in the staged
 * write's dialog as a DJ does it: a Spare stick with its button, a Gig
 * stick by typing its name (shown in the dialog's own words).
 */
export async function unlockInDialog(page: Page): Promise<void> {
  const { expect } = await import("@playwright/test");
  const dialog = page.getByRole("alertdialog", { name: /^Write to / });
  const unlock = dialog.locator('[data-stick-protection="protected"]');
  if (await unlock.count() === 0) return;
  const field = unlock.getByRole("textbox");
  if (await field.count() > 0) {
    const label = await field.getAttribute("aria-label") ?? "";
    const name = /^Type (.+) to unlock it$/.exec(label)?.[1] ?? "";
    await field.fill(name);
  }
  await unlock.getByRole("button", { name: /^Unlock / }).click();
  await expect(dialog.locator('[data-stick-protection="unlocked"]')).toBeVisible();
}

export async function confirmStickWrite(page: Page): Promise<void> {
  const { expect } = await import("@playwright/test");
  const dialog = page.getByRole("alertdialog", { name: /^Write to / });
  await expect(dialog).toBeVisible();
  // This request's dialog: another may follow it at once (a sync, one per Stick).
  const what = (await dialog.locator("#stick-write-what p").first().innerText()).trim();
  await unlockInDialog(page);
  await dialog.getByRole("button", { name: /Hold to write/ }).hover();
  await page.mouse.down();
  await expect(page.getByRole("alertdialog").filter({ hasText: what })).toHaveCount(0, { timeout: 6000 });
  await page.mouse.up();
}
