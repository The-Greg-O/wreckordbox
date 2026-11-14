/**
 * Preferences' pane rail: its icons read under every theme (wbx-4g6). The
 * rail keeps upstream's dark look under a light theme (wbx-lfa); what is
 * read back is each icon's colour against the ground painted behind it
 * (`lightAudit.ts`): the search field's magnifier on its black well, the
 * unlit panes' icons, and the selected pane's icon on the selection.
 */
import { expect, test } from "@playwright/test";

import { open } from "./compactPlayer";
import { audit, shortfalls } from "./lightAudit";
import { LATTE, TOKYO_NIGHT, WHITE, announce } from "./lightPalettes";

test("Preferences' pane rail icons can be read under the default, a dark and the light themes", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Shift+Control+P");
  await page.getByLabel("Search actions").fill("Preferences");
  await page.keyboard.press("Enter");
  const rail = page.getByRole("navigation", { name: "Preference panes" });
  await expect(rail).toBeVisible();
  for (const [name, roles] of [["the default theme", null], ["Tokyo Night", TOKYO_NIGHT], ["Catppuccin Latte", LATTE], ["White", WHITE]] as const) {
    if (roles !== null) await announce(page, { roles, font: null });
    const icons = (await audit(rail)).seen.filter((s) => s.kind === "icon");
    // The magnifier and every pane's icon, one of them selected.
    expect(icons.length, name).toBeGreaterThanOrEqual(8);
    expect([...new Set(shortfalls(icons))], name).toEqual([]);
  }
});
