/**
 * The chrome follows the Omarchy theme; what the music means does not.
 *
 * The mock interface has no app shell, so it starts on the stylesheet's
 * fallbacks, upstream's own colours and face. A theme is then announced to
 * the page the way the app shell announces one, as an event that
 * `followTheme()` (started by `main.tsx`) hears and `applyTheme()` shows;
 * nothing here writes a style itself. What is read back is what real
 * controls are drawn in.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";
import { BROWSER_THEME_EVENT, type Theme, type ThemeRoles } from "../src/lib/themeRoles";

const TOKYO_NIGHT: ThemeRoles = {
  mode: "dark", ground: "#0e0e14", panel: "#13141c", working: "#1a1b26", hairline: "#24283b",
  selection: "#292e42", border: "#414868", text: "#a9b1d6", textDim: "#565f89", textBright: "#c0caf5",
  accent: "#7aa2f7", onAccent: "#0e0e14", tabFrom: "#bb9af7", tabTo: "#ff9e64",
};
const GRUVBOX: ThemeRoles = {
  mode: "dark", ground: "#1d2021", panel: "#222526", working: "#282828", hairline: "#3c3836",
  selection: "#504945", border: "#665c54", text: "#d4be98", textDim: "#a89984", textBright: "#ddc7a1",
  accent: "#7daea3", onAccent: "#1d2021", tabFrom: "#d3869b", tabTo: "#d8a657",
};
/** What the app shell derives from a file with only `background = #101010` and `foreground = #f0f0f0`: the accent is the text colour. */
const TWO_COLOURS: ThemeRoles = {
  mode: "dark", ground: "#090909", panel: "#0c0c0c", working: "#101010", hairline: "#262626",
  selection: "#383838", border: "#535353", text: "#f0f0f0", textDim: "#8b8b8b", textBright: "#f0f0f0",
  accent: "#f0f0f0", onAccent: "#090909", tabFrom: "#f0f0f0", tabTo: "#f0f0f0",
};
const LATTE: ThemeRoles = {
  mode: "light", ground: "#d7d8dc", panel: "#e3e4e8", working: "#eff1f5", hairline: "#dce0e8",
  selection: "#ccd0da", border: "#acb0be", text: "#4c4f69", textDim: "#9ca0b0", textBright: "#4c4f69",
  accent: "#1e66f5", onAccent: "#ffffff", tabFrom: "#8839ef", tabTo: "#df8e1d",
};
const FACE = "JetBrainsMono Nerd Font";

/** Colours that carry meaning: bands, phrases, cues, Prep colours, meters, the playhead, Energy. */
const MEANING = [
  "--c-wave-low", "--c-wave-all", "--c-phrase-chorus", "--c-cue-hot", "--c-cue-purple", "--c-label-pink",
  "--c-comment-red", "--c-meter-green", "--c-playhead", "--c-downbeat", "--c-played", "--c-energy-0", "--c-energy-3", "--c-energy-5", "--c-loop-band",
];

/** Each token as the browser resolves it on a real element, so `var()` chains are followed. */
async function resolved(page: Page, names: string[]): Promise<Record<string, string>> {
  return page.evaluate((tokens) => {
    const probe = document.createElement("i");
    document.body.append(probe);
    const found: Record<string, string> = {};
    for (const name of tokens) {
      probe.style.color = `var(${name})`;
      found[name] = getComputedStyle(probe).color;
    }
    probe.remove();
    return found;
  }, names);
}

/** Announces `theme` to the page as the app shell would. */
async function announce(page: Page, theme: Theme): Promise<void> {
  await page.evaluate(
    ([name, detail]) => void window.dispatchEvent(new CustomEvent(name as string, { detail })),
    [BROWSER_THEME_EVENT, theme] as const,
  );
}

const rgb = (hex: string): string => {
  const n = Number.parseInt(hex.slice(1), 16);
  return `rgb(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255})`;
};

const drawn = (target: Locator, property: string): Promise<string> =>
  target.evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), property);

/** WCAG contrast of two `rgb(r, g, b)` colours. */
function contrast(one: string, other: string): number {
  const luminance = (colour: string) => {
    const [r, g, b] = (colour.match(/\d+(\.\d+)?/g) ?? []).slice(0, 3).map((value) => {
      const channel = Number(value) / 255;
      return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
  };
  const [high, low] = [luminance(one), luminance(other)].sort((a, b) => b - a);
  return (high! + 0.05) / (low! + 0.05);
}

async function open(page: Page): Promise<void> {
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)");
}

const treeRow = (page: Page): Locator => page.getByRole("treeitem").filter({ hasText: "Melodic Vox" }).first();
const memoryTab = (page: Page): Locator => page.getByRole("tab", { name: "MEMORY" }).first();

test("without an app shell the interface keeps upstream's colours and face", async ({ page }) => {
  await open(page);
  await expect(page.locator("html")).not.toHaveAttribute("data-theme-mode", /.*/);
  expect(await drawn(treeRow(page), "color")).toBe(rgb("#E6E6E6"));
  expect(await drawn(page.locator("body"), "font-family")).toContain("Arial");
  // The lit player tab is upstream's measured blue with upstream's near-white on it.
  expect(await drawn(memoryTab(page), "background-color")).toBe(rgb("#457FE5"));
  expect(await drawn(memoryTab(page), "color")).toBe(rgb("#F0F0F0"));
});

test("a theme announced to the page recolours the chrome, and a second one replaces it without a reload", async ({ page }) => {
  await open(page);
  const meaning = await resolved(page, MEANING);

  await announce(page, { roles: TOKYO_NIGHT, font: FACE });
  await expect(page.locator("html")).toHaveAttribute("data-theme-mode", "dark");
  expect(await drawn(treeRow(page), "color")).toBe(rgb(TOKYO_NIGHT.text));
  expect(await drawn(page.locator("body"), "font-family")).toContain(FACE);
  expect((await resolved(page, ["--c-panel"]))["--c-panel"]).toBe(rgb(TOKYO_NIGHT.working));
  expect(await resolved(page, MEANING)).toEqual(meaning);

  await announce(page, { roles: GRUVBOX, font: FACE });
  expect(await drawn(treeRow(page), "color")).toBe(rgb(GRUVBOX.text));
  expect((await resolved(page, ["--c-select"]))["--c-select"]).toBe(rgb(GRUVBOX.selection));
  expect(await resolved(page, MEANING)).toEqual(meaning);

  // The theme goes away: back to upstream's whole.
  await announce(page, { roles: null, font: null });
  await expect(page.locator("html")).not.toHaveAttribute("data-theme-mode", /.*/);
  expect(await drawn(treeRow(page), "color")).toBe(rgb("#E6E6E6"));
});

test("Energy stars are drawn in Energy's own level colours under two themes, at rest and under the pointer", async ({ page }) => {
  await open(page);
  // A rated track: its lit stars are its level's colour, its unlit ones the unset colour.
  const group = page.getByRole("radiogroup", { name: "Rating" }).filter({ has: page.getByRole("radio", { checked: true }) }).first();
  const level = Number((await group.getByRole("radio", { checked: true }).getAttribute("aria-label"))!.split(" ")[0]);
  expect(level).toBeGreaterThan(0);
  expect(level).toBeLessThan(5);
  const icon = (star: number) => group.getByRole("radio").nth(star - 1).locator("svg");
  const tokens = await resolved(page, ["--c-energy-0", `--c-energy-${level}`, "--c-energy-5"]);
  const seen: string[][] = [];
  for (const roles of [null, TOKYO_NIGHT, GRUVBOX]) {
    await announce(page, { roles, font: null });
    await page.mouse.move(0, 0);
    const lit = await drawn(icon(1), "color");
    const unlit = await drawn(icon(5), "color");
    await group.getByRole("radio").nth(4).hover();
    seen.push([lit, unlit, await drawn(icon(5), "color")]);
  }
  // Under the pointer the fifth star shows the level it would set.
  expect(seen[0]).toEqual([tokens[`--c-energy-${level}`], tokens["--c-energy-0"], tokens["--c-energy-5"]]);
  expect(seen[1]).toEqual(seen[0]);
  expect(seen[2]).toEqual(seen[0]);
});

test("the lit states of the chrome take the accent, with a label that can be read on it", async ({ page }) => {
  await open(page);
  for (const roles of [TOKYO_NIGHT, GRUVBOX, TWO_COLOURS, LATTE]) {
    await announce(page, { roles, font: null });
    const fill = await drawn(memoryTab(page), "background-color");
    const label = await drawn(memoryTab(page), "color");
    expect(fill, roles.accent).toBe(rgb(roles.accent));
    expect(label, roles.accent).toBe(rgb(roles.onAccent));
    expect(contrast(fill, label), roles.accent).toBeGreaterThanOrEqual(4.5);
  }
});

test("a hovered menu entry can be read when the theme's accent is its text colour", async ({ page }) => {
  await open(page);
  await announce(page, { roles: TWO_COLOURS, font: null });
  await treeRow(page).click({ button: "right" });
  const menu = page.getByRole("menu", { name: "Playlist" });
  await expect(menu).toBeVisible();
  const entry = menu.getByRole("menuitem", { name: "Export Playlist" });
  await entry.hover();
  const fill = await drawn(entry, "background-color");
  expect(fill).toBe(rgb(TWO_COLOURS.accent));
  expect(contrast(fill, await drawn(entry, "color"))).toBeGreaterThanOrEqual(4.5);

  // And under a full theme whose accent upstream's near-white label would
  // have sat on at about 2:1.
  await announce(page, { roles: TOKYO_NIGHT, font: null });
  await entry.hover();
  expect(contrast(await drawn(entry, "background-color"), await drawn(entry, "color"))).toBeGreaterThanOrEqual(4.5);
});

test("under a light theme upstream's layout stays dark and readable, and takes the accent", async ({ page }) => {
  await open(page);
  await announce(page, { roles: LATTE, font: null });
  await expect(page.locator("html")).toHaveAttribute("data-theme-mode", "light");
  const text = await drawn(treeRow(page), "color");
  expect(text).toBe(rgb("#E6E6E6"));
  const grounds = await resolved(page, ["--c-black", "--c-top", "--c-panel", "--c-select"]);
  for (const ground of Object.values(grounds)) expect(contrast(ground, text)).toBeGreaterThanOrEqual(4.5);
  expect((await resolved(page, ["--c-accent"]))["--c-accent"]).toBe(rgb(LATTE.accent));
});

test("the view shell's active tab carries the theme's two-colour line", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/");
  const tab = page.getByRole("tablist", { name: "Views" }).getByRole("tab", { selected: true });
  await expect(tab).toBeVisible();
  // Without a theme both ends are upstream's accent: a plain line.
  expect(await drawn(tab, "border-image-source")).toBe(`linear-gradient(90deg, ${rgb("#1373EB")}, ${rgb("#1373EB")})`);
  await announce(page, { roles: TOKYO_NIGHT, font: FACE });
  expect(await drawn(tab, "border-image-source")).toBe(
    `linear-gradient(90deg, ${rgb(TOKYO_NIGHT.tabFrom)}, ${rgb(TOKYO_NIGHT.tabTo)})`,
  );
  expect(await drawn(tab, "color")).toBe(rgb(TOKYO_NIGHT.text));
  expect(await drawn(page.getByTestId("shell-top-bar"), "font-family")).toContain(FACE);
});

test("in the view shell the chrome follows two dark themes and Energy, Prep and the player's phrases do not", async ({ page }) => {
  await enableViewShell(page);
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  const list = page.getByRole("tabpanel");
  const pips = list.getByRole("radiogroup", { name: "Energy 3 of 5" }).first().getByRole("radio");
  const stripe = list.getByRole("img", { name: "Prep colour: Pink" }).first();
  const tree = page.getByRole("tree", { name: "Tree" });
  const treeItem = tree.getByRole("treeitem").first();
  await expect(pips.first()).toBeVisible();
  const background = (target: Locator) => drawn(target, "background-color");
  /** The nearest painted ground behind `target`. */
  const ground = (target: Locator) =>
    target.evaluate((el) => {
      for (let at: Element | null = el; at; at = at.parentElement) {
        const colour = getComputedStyle(at).backgroundColor;
        if (colour !== "rgba(0, 0, 0, 0)" && colour !== "transparent") return colour;
      }
      return getComputedStyle(document.body).backgroundColor;
    });
  const meaning = async () => [
    await background(pips.nth(0)), await background(pips.nth(4)), await background(stripe),
    ...Object.values(await resolved(page, MEANING)),
  ];

  const seen: string[][] = [];
  for (const roles of [TOKYO_NIGHT, GRUVBOX]) {
    await announce(page, { roles, font: FACE });
    seen.push(await meaning());
    // The chrome is the theme's, and its text can be read on it.
    expect(await drawn(treeItem, "color"), roles.text).toBe(rgb(roles.text));
    expect(contrast(await drawn(treeItem, "color"), await ground(treeItem)), roles.text).toBeGreaterThanOrEqual(4.5);
    // A lit pip stands out from the row it sits in.
    expect(contrast(await background(pips.nth(0)), await ground(list.locator('[role="row"][aria-selected]').first())))
      .toBeGreaterThanOrEqual(3);
  }
  expect(seen[1]).toEqual(seen[0]);
  expect(seen[0]!.slice(0, 2)).toEqual(Object.values(await resolved(page, ["--c-energy-3", "--c-energy-0"])));
});
