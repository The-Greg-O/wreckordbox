import { expect, test, type Page } from "@playwright/test";
import { seedShell, player, playButton } from "./compactPlayer";
import { BROWSER_THEME_EVENT, type Theme } from "../src/lib/themeRoles";

const row = (page: Page, n: number) => page.getByRole("tabpanel").getByRole("row").filter({
  has: page.getByRole("gridcell", { name: `Key light ${String(n).padStart(2, "0")}`, exact: true }),
});
const key = (page: Page, n: number) => row(page, n).locator('[data-col="key"]');

async function open(page: Page, protect = false, view: Record<string, unknown> = {}) {
  await seedShell(page, view);
  if (protect) await page.addInitScript(() => {
    const prefs = JSON.parse(localStorage.getItem("rbl.preferences") ?? "{}");
    prefs.advanced = { protectLibrary: true };
    localStorage.setItem("rbl.preferences", JSON.stringify(prefs));
  });
  await page.goto("/?writable=1&keyLights=1");
  await page.getByRole("treeitem", { name: "All Tracks", exact: true }).click();
  await expect(row(page, 1)).toBeVisible();
}

async function load(page: Page, n: number) {
  await row(page, n).locator('[data-col="title"]').dblclick();
  await expect(player(page).getByTestId("compact-player-title")).toHaveText(`Key light ${String(n).padStart(2, "0")}`);
}

// Assert the colour actually painted, not the presence of a state attribute.
async function light(page: Page, n: number, colour: "green" | "yellow" | "none") {
  const badge = key(page, n).locator("span").first();
  await expect(badge).toHaveCSS("background-color", {
    green: "rgb(158, 206, 106)", yellow: "rgb(224, 175, 104)", none: "rgba(0, 0, 0, 0)",
  }[colour]);
}

test("Camelot is the default; Classic changes the list and loaded player and persists", async ({ page }) => {
  await open(page);
  await expect(key(page, 1)).toHaveText("8A");
  await load(page, 1);
  await expect(player(page).getByTestId("compact-player-key")).toHaveText("8A");
  await page.getByRole("button", { name: "Command palette", exact: true }).click();
  await page.getByRole("combobox", { name: "Search actions" }).fill("preferences");
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Preferences" });
  const choices = dialog.getByRole("radiogroup", { name: "Key display format", exact: true });
  await expect(choices.getByRole("radio", { name: "Camelot", exact: true })).toBeChecked();
  const classic = choices.getByRole("radio", { name: "Classic", exact: true });
  await classic.focus();
  await page.keyboard.press("Space");
  await page.keyboard.press("Enter");
  await expect(classic).toBeChecked();
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(key(page, 1)).toHaveText("Am");
  await expect(player(page).getByTestId("compact-player-key")).toHaveText("Am");
  await expect(playButton(page)).toBeVisible();
  await light(page, 5, "yellow");
  await page.reload();
  await expect(key(page, 1)).toHaveText("Am");
});

test("lights follow the loaded key, never row selection, and wrap the wheel", async ({ page }) => {
  await open(page);
  await expect(key(page, 1)).toHaveText("8A");
  for (let n = 1; n <= 12; n++) await light(page, n, "none");
  await load(page, 1);
  for (const n of [1, 2, 3, 4]) await light(page, n, "green");
  await light(page, 5, "yellow");
  for (const n of [6, 7, 8, 9, 10, 11, 12]) await light(page, n, "none");
  await row(page, 5).locator('[data-col="title"]').click();
  await light(page, 5, "yellow");
  await light(page, 1, "green");
  await load(page, 5);
  await light(page, 1, "none");
  await light(page, 3, "green");
  await light(page, 11, "yellow");
  await load(page, 10); // 11A → 1A wraps for the +2 energy boost.
  await light(page, 12, "yellow");
  await load(page, 8); // Missing loaded key clears everything.
  for (let n = 1; n <= 12; n++) await light(page, n, "none");
});

test("Library Protection refuses a key edit while notation and lights still work", async ({ page }) => {
  await open(page, true);
  await load(page, 1);
  await key(page, 5).dblclick();
  await expect(page.getByTestId("shell-status")).toContainText("Editing is locked by Library Protection");
  await expect(key(page, 5)).toHaveText("10A");
  await light(page, 5, "yellow");
  await expect(page.getByRole("combobox", { name: "Key", exact: true })).toHaveCount(0);
});

test("editing a displayed Camelot key saves the stored key through the existing edit and Undo", async ({ page }) => {
  await open(page);
  await load(page, 1);
  await key(page, 6).dblclick();
  const editor = page.getByRole("combobox", { name: "Key", exact: true });
  await editor.selectOption({ label: "10A" });
  await expect(key(page, 6)).toHaveText("10A");
  await light(page, 6, "yellow");
  await expect(player(page).getByTestId("compact-player-key")).toHaveText("8A");
  await expect(playButton(page)).toBeVisible();
  await page.evaluate(() => (window as unknown as { __menu: (id: string) => void }).__menu("undo"));
  await expect(key(page, 6)).toHaveText("6A");
  await light(page, 6, "none");
});

const LIGHT: Theme = {
  font: null,
  roles: {
    mode: "light", ground: "#d7d8dc", panel: "#e3e4e8", working: "#eff1f5", hairline: "#dce0e8",
    selection: "#ccd0da", border: "#acb0be", text: "#4c4f69", textDim: "#9ca0b0", textBright: "#4c4f69",
    accent: "#1e66f5", onAccent: "#ffffff", tabFrom: "#8839ef", tabTo: "#df8e1d",
  },
};

for (const width of [1280, 640, 436]) {
  for (const uiScale of [4, 6]) {
    test(`key lights stay legible at ${width}px, scale ${uiScale}, and in a light theme`, async ({ page }, info) => {
      await open(page, false, { uiScale });
      await load(page, 1);
      // Choose All Tracks while the tree is on screen, then tile the window.
      await page.setViewportSize({ width, height: 900 });
      // wbx-60 owns dropping columns at narrow widths. Until then the
      // existing list scrolls sideways; check our labels once in view.
      await key(page, 5).scrollIntoViewIfNeeded();
      for (const theme of ["dark", "light"]) {
        if (theme === "light") await page.evaluate(([event, detail]) => {
          window.dispatchEvent(new CustomEvent(event, { detail }));
        }, [BROWSER_THEME_EVENT, LIGHT] as const);
        if (theme === "light") await expect(page.locator("html")).toHaveAttribute("data-theme-mode", "light");
        await light(page, 1, "green");
        await light(page, 5, "yellow");
        for (const n of [1, 5]) {
          const badge = key(page, n).locator("span").first();
          await expect(badge).toHaveCSS("color", "rgb(14, 14, 20)");
          const box = await badge.boundingBox();
          const cell = await key(page, n).boundingBox();
          expect(box).not.toBeNull();
          expect(box!.x).toBeGreaterThanOrEqual(cell!.x);
          expect(box!.x + box!.width).toBeLessThanOrEqual(Math.min(width, cell!.x + cell!.width));
        }
        await page.screenshot({ path: info.outputPath(`keys-${theme}.png`) });
      }
    });
  }
}
