/**
 * Light Omarchy themes reach what the view shell opens outside its frame
 * (wbx-lfa): context menus, the command palette and dialogs, the staged
 * Stick write with its unlock among them. They render outside the shell's
 * role scope (portals on the body, the window's dialogs beside the shell),
 * so they used to stay dark. Upstream's old layout keeps all of them dark,
 * as it keeps itself (wbx-47, until wbx-77).
 *
 * What is read back is what each surface is drawn in: every text and icon
 * against the ground actually painted behind it (`lightAudit.ts`), and the
 * surface's own ground.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { audit, shortfalls } from "./lightAudit";
import { open } from "./compactPlayer";
import { enableViewShell } from "./helpers";
import { LATTE, WHITE, announce, rgb } from "./lightPalettes";

test.describe.configure({ timeout: 120_000 });

const LIGHT = [LATTE, WHITE];
const groundsOf = (roles: typeof LATTE) => new Set([roles.ground, roles.panel, roles.working, roles.hairline, roles.selection].map(rgb));

/** The ground a surface is drawn on: the first opaque background from it outwards. */
const surfaceGround = (surface: Locator): Promise<string> => surface.evaluate((el) => {
  for (let at: Element | null = el; at; at = at.parentElement) {
    const c = getComputedStyle(at).backgroundColor;
    if (c !== "rgba(0, 0, 0, 0)" && !c.endsWith(", 0)")) return c;
  }
  return "";
});

/** Under each light theme the surface reads at 4.5:1 (3:1 for secondary text) and is drawn on one of the theme's grounds. */
async function readable(page: Page, surface: () => Locator, what: string): Promise<void> {
  for (const roles of LIGHT) {
    await announce(page, { roles, font: null });
    await expect(surface()).toBeVisible();
    const { seen } = await audit(surface());
    expect(seen.length, `${what}: something was read`).toBeGreaterThan(1);
    expect([...new Set(shortfalls(seen))], `${what} under ${roles.working}`).toEqual([]);
    expect(groundsOf(roles).has(await surfaceGround(surface())), `${what} ground under ${roles.working}`).toBe(true);
  }
}

const palette = (page: Page) => page.getByRole("dialog", { name: "Command palette" });
const tree = (page: Page) => page.getByRole("tree", { name: "Tree" });
const stickTest = (page: Page) => tree(page).getByRole("group", { name: "Stick TEST", exact: true });

async function openPalette(page: Page): Promise<void> {
  await page.getByRole("button", { name: "Command palette" }).click();
  await expect(palette(page)).toBeVisible();
}

test("under a light theme the command palette, with an entry chosen and a greyed one, can be read on the theme's grounds", async ({ page }) => {
  await open(page);
  await openPalette(page);
  // A greyed entry is among the first ones shown (a deck command with nothing loaded).
  await expect(palette(page).getByRole("option", { disabled: true }).first()).toBeVisible();
  await palette(page).getByLabel("Search actions").fill("st");
  await readable(page, () => palette(page), "command palette");
});

test("under a light theme a track's and a playlist's context menu, with an entry under the pointer, can be read on the theme's grounds", async ({ page }) => {
  await open(page);
  await page.locator('[role="gridcell"][data-col="title"]').nth(1).click({ button: "right" });
  const menu = () => page.getByRole("menu").first();
  await expect(menu()).toBeVisible();
  await menu().getByRole("menuitem").first().hover();
  await readable(page, menu, "track menu");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);

  await tree(page).getByRole("treeitem").filter({ hasText: "Melodic Vox" }).first().click({ button: "right" });
  await expect(menu()).toBeVisible();
  await readable(page, menu, "playlist menu");
});

test("under a light theme the Stick Protection dialog with its unlock can be read on the theme's grounds", async ({ page }) => {
  await open(page);
  await openPalette(page);
  await palette(page).getByLabel("Search actions").fill("Stick Protection");
  await page.keyboard.press("Enter");
  const dialog = () => page.getByRole("dialog", { name: "Stick Protection" });
  await expect(dialog().locator('[data-stick-protection="protected"]').first()).toBeVisible();
  await readable(page, dialog, "Stick Protection dialog");
});

test("under a light theme the staged Stick write, protected and then unlocked, can be read on the theme's grounds", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/?writable=1");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
  await stickTest(page).getByRole("treeitem", { name: "TEST", exact: true }).click({ button: "right" });
  await page.getByRole("menu", { name: "Playlists" }).getByRole("menuitem", { name: "Create New Playlist" }).click();
  const dialog = () => page.getByRole("alertdialog", { name: /^Write to / });
  // Protected: the unlock, a Gig stick's typed name.
  await expect(dialog().locator('[data-stick-protection="protected"]')).toBeVisible();
  await readable(page, dialog, "Stick write, protected");
  const field = dialog().getByRole("textbox");
  await field.fill("TEST");
  await dialog().getByRole("button", { name: /^Unlock / }).click();
  await expect(dialog().locator('[data-stick-protection="unlocked"]')).toBeVisible();
  // Unlocked: the hold button and what it will write.
  await readable(page, dialog, "Stick write, unlocked");
  await dialog().getByRole("button", { name: "Cancel" }).click();
  await expect(dialog()).toHaveCount(0);
});

test("upstream's old layout keeps its menus, palette and dialogs dark under a light theme", async ({ page }) => {
  await open(page, "", { viewShell: false });
  await expect(page.getByTestId("shell")).toHaveCount(0);
  await announce(page, { roles: LATTE, font: null });
  await page.getByRole("button", { name: "Command palette" }).click();
  await expect(palette(page)).toBeVisible();
  expect(await surfaceGround(palette(page))).toBe(rgb("#1E1E1E"));
  await page.keyboard.press("Escape");
  await page.locator('[role="gridcell"][data-col="title"]').nth(1).click({ button: "right" });
  const menu = page.getByRole("menu").first();
  await expect(menu).toBeVisible();
  expect(await surfaceGround(menu)).toBe(rgb("#1E1E1E"));
});
