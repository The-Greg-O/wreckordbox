/**
 * The Gig stick unlock's name field can be seen and used (wbx-cbh): the DJ
 * is asked to type the Stick's name, so the field has a ground, an edge
 * that stands out from what is around it, a placeholder, a caret and a
 * focus ring, under dark and light themes and at the 0.8, 1 and 1.25 scales,
 * in the staged write's dialog (where it takes the focus) and in the Stick
 * view's in-place unlock. Typing in it never reaches the deck.
 *
 * What is read back is what is painted: the field's measured box and the
 * contrast of each part against the ground actually behind it.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";

import { open, pauseButton, player, titles } from "./compactPlayer";
import { LATTE, TOKYO_NIGHT, WHITE, announce } from "./lightPalettes";

test.describe.configure({ timeout: 120_000 });

type Look = {
  width: number; height: number; inside: boolean;
  edge: number; text: number; placeholder: number; placeholderText: string; caret: number;
};

/** Measures the field: its box, and each part's contrast against the ground it is drawn on. */
const look = (field: Locator): Promise<Look> => field.evaluate((el) => {
  const input = el as HTMLInputElement;
  type C = [number, number, number, number];
  // `rgb()`/`rgba()`, or `color(srgb …)` (0 to 1) as Chromium reports a `color-mix()`.
  const parse = (css: string): C => {
    const m = /^(rgba?|color)\((?:srgb\s+)?([^)]+)\)$/.exec(css);
    if (!m) throw new Error(`unreadable colour ${css}`);
    const [r, g, b, a] = m[2]!.split(/[\s,/]+/).filter(Boolean).map(Number);
    const k = m[1] === "color" ? 255 : 1;
    return [r! * k, g! * k, b! * k, a ?? 1];
  };
  const over = (top: C, under: C): C => {
    const a = top[3];
    return [top[0] * a + under[0] * (1 - a), top[1] * a + under[1] * (1 - a), top[2] * a + under[2] * (1 - a), 1];
  };
  const lum = (c: C) => {
    const [r, g, b] = c.slice(0, 3).map((v) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; });
    return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
  };
  const contrast = (a: C, b: C) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x! + 0.05) / (y! + 0.05); };
  // The ground around the field: the first opaque background from its parent outwards.
  let around: C = [0, 0, 0, 1];
  for (let at = input.parentElement; at; at = at.parentElement) {
    const c = parse(getComputedStyle(at).backgroundColor);
    if (c[3] > 0) { around = c[3] < 1 ? over(c, [0, 0, 0, 1]) : c; break; }
  }
  const style = getComputedStyle(input);
  const ground = over(parse(style.backgroundColor), around);
  // The edge as laid out: under the scale's CSS zoom the computed width reads below 1px while 1px is painted.
  const edgeWidth = (input.offsetHeight - input.clientHeight) / 2;
  const edgeColour = over(parse(style.borderBottomColor), around);
  // The field is told from its surroundings by its edge or by its own ground.
  const edge = Math.max(edgeWidth >= 1 ? contrast(edgeColour, around) : 1, contrast(ground, around));
  const caretCss = style.caretColor === "auto" ? style.color : style.caretColor;
  const box = input.getBoundingClientRect();
  const frame = input.closest('[role="alertdialog"], [role="dialog"], [data-testid="stick-view"]')!.getBoundingClientRect();
  return {
    width: box.width,
    height: box.height,
    inside: box.left >= frame.left && box.right <= frame.right && box.top >= frame.top && box.bottom <= frame.bottom,
    edge,
    text: contrast(over(parse(style.color), ground), ground),
    placeholder: contrast(over(parse(getComputedStyle(input, "::placeholder").color), ground), ground),
    placeholderText: input.placeholder,
    caret: contrast(over(parse(caretCss), ground), ground),
  };
});

/** The focus ring, read with the field focused: its width and its contrast against the ground around it. */
const ring = (field: Locator) => field.evaluate((el) => {
  type C = [number, number, number];
  const parse = (css: string): C => {
    const m = /^(rgba?|color)\((?:srgb\s+)?([^)]+)\)$/.exec(css)!;
    const k = m[1] === "color" ? 255 : 1;
    return m[2]!.split(/[\s,/]+/).filter(Boolean).slice(0, 3).map((v) => Number(v) * k) as C;
  };
  const lum = (c: C) => {
    const [r, g, b] = c.map((v) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; });
    return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
  };
  let around: C = [0, 0, 0];
  for (let at = el.parentElement; at; at = at.parentElement) {
    const bg = getComputedStyle(at).backgroundColor;
    if (!bg.endsWith(", 0)") && bg !== "transparent") { around = parse(bg); break; }
  }
  const style = getComputedStyle(el);
  const width = style.outlineStyle === "none" ? 0 : Number.parseFloat(style.outlineWidth);
  const [x, y] = [lum(parse(style.outlineColor)), lum(around)].sort((p, q) => q - p);
  return { width, contrast: (x! + 0.05) / (y! + 0.05) };
});

const THEMES = [
  { name: "the default dark theme", roles: null },
  { name: "Tokyo Night", roles: TOKYO_NIGHT },
  { name: "Catppuccin Latte", roles: LATTE },
  { name: "White", roles: WHITE },
] as const;

/** Under every theme the field is a box the DJ can see, type into and find again when focused. */
async function visibleUnderEveryTheme(page: Page, field: () => Locator, scale: number, where: string): Promise<void> {
  for (const theme of THEMES) {
    if (theme.roles !== null) await announce(page, { roles: theme.roles, font: null });
    const what = `${where}, ${theme.name}, scale ${scale}`;
    await field().blur();
    const seen = await look(field());
    expect(seen.width, `${what}: width`).toBeGreaterThanOrEqual(100 * scale);
    expect(seen.height, `${what}: height`).toBeGreaterThanOrEqual(16 * scale);
    expect(seen.inside, `${what}: inside its frame`).toBe(true);
    expect(seen.edge, `${what}: edge against the ground`).toBeGreaterThanOrEqual(3);
    expect(seen.text, `${what}: typed text`).toBeGreaterThanOrEqual(4.5);
    expect(seen.placeholderText, `${what}: placeholder`).not.toBe("");
    expect(seen.placeholder, `${what}: placeholder`).toBeGreaterThanOrEqual(3);
    expect(seen.caret, `${what}: caret`).toBeGreaterThanOrEqual(3);
    await field().focus();
    const focused = await ring(field());
    expect(focused.width, `${what}: focus ring`).toBeGreaterThanOrEqual(1);
    expect(focused.contrast, `${what}: focus ring against the ground`).toBeGreaterThanOrEqual(3);
  }
}

const tree = (page: Page) => page.getByRole("tree", { name: "Tree" });
const stickTest = (page: Page) => tree(page).getByRole("group", { name: "Stick TEST", exact: true });
const writeDialog = (page: Page) => page.getByRole("alertdialog", { name: /^Write to / });

for (const [stop, scale] of [[2, 0.8], [4, 1], [6, 1.25]] as const) {
  test(`at scale ${scale} the staged write's Gig stick name field takes the focus and can be seen under dark and light themes`, async ({ page }) => {
    await open(page, "?writable=1", { uiScale: stop });
    await stickTest(page).getByRole("treeitem", { name: "TEST", exact: true }).click({ button: "right" });
    await page.getByRole("menu", { name: "Playlists" }).getByRole("menuitem", { name: "Create New Playlist" }).click();
    const field = () => writeDialog(page).getByRole("textbox", { name: "Type TEST to unlock it" });
    // The dialog asks for the name: the field has the focus, and what is typed lands in it.
    await expect(field()).toBeFocused();
    await page.keyboard.type("TE");
    await expect(field()).toHaveValue("TE");
    await visibleUnderEveryTheme(page, field, scale, "staged write");
    // Still keyboard-operable: the name typed and Enter unlocks it.
    await field().focus();
    await page.keyboard.type("ST");
    await page.keyboard.press("Enter");
    await expect(writeDialog(page).locator('[data-stick-protection="unlocked"]')).toBeVisible();
  });
}

test("the Stick view's Gig stick name field can be seen under dark and light themes, and typing in it never reaches the deck", async ({ page }) => {
  await open(page);
  await titles(page).nth(3).dblclick();
  await page.getByRole("tablist", { name: "Views" }).getByRole("tab", { name: "Stick" }).click();
  const field = () => page.getByTestId("stick-view").getByRole("textbox", { name: /^Type .+ to unlock it$/ });
  await visibleUnderEveryTheme(page, field, 1, "Stick view");
  await field().focus();
  await page.keyboard.type("DJ STICK");
  await page.keyboard.press("Space");
  await expect(field()).toHaveValue("DJ STICK ");
  await expect(pauseButton(page)).toHaveCount(0);
  await expect(player(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();
});
