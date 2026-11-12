/**
 * The window's own theme file (`~/.config/wreckordbox/theme.toml`) over the
 * Omarchy theme.
 *
 * The app shell lays the file over the Omarchy palette and derives the roles
 * (`src-tauri/src/omarchy_theme.rs`, unit-tested there); what reaches the
 * window is the same `Theme` as before, with the file's faces, sizes and
 * mistakes added. The mock interface has no app shell, so the theme is
 * announced to the page as the app shell announces one, and what is read
 * back is what real controls are drawn in.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";
import { enableViewShell } from "./helpers";
import { BROWSER_THEME_EVENT, type Theme, type ThemeRoles } from "../src/lib/themeRoles";

const TOKYO_NIGHT: ThemeRoles = {
  mode: "dark", ground: "#0e0e14", panel: "#13141c", working: "#1a1b26", hairline: "#24283b",
  selection: "#292e42", border: "#414868", text: "#a9b1d6", textDim: "#565f89", textBright: "#c0caf5",
  accent: "#7aa2f7", onAccent: "#0e0e14", tabFrom: "#bb9af7", tabTo: "#ff9e64",
};
const FACE = "JetBrainsMono Nerd Font";
/** Tokyo Night with a theme file that says `foreground = "#ffcc00"`, as the app shell lays it over. */
const TEXT_OVERRIDDEN: ThemeRoles = { ...TOKYO_NIGHT, text: "#ffcc00" };

/** Colours that carry meaning, which no theme file reaches. */
const MEANING = [
  "--c-wave-low", "--c-wave-all", "--c-phrase-chorus", "--c-cue-hot", "--c-label-pink", "--c-comment-red",
  "--c-meter-green", "--c-playhead", "--c-energy-0", "--c-energy-1", "--c-energy-3", "--c-energy-5", "--c-loop-band",
];

async function announce(page: Page, theme: Theme): Promise<void> {
  await page.evaluate(
    ([name, detail]) => void window.dispatchEvent(new CustomEvent(name as string, { detail })),
    [BROWSER_THEME_EVENT, theme] as const,
  );
}

async function resolved(page: Page, names: string[]): Promise<string[]> {
  return page.evaluate((tokens) => {
    const probe = document.createElement("i");
    document.body.append(probe);
    const found = tokens.map((name) => {
      probe.style.color = `var(${name})`;
      return getComputedStyle(probe).color;
    });
    probe.remove();
    return found;
  }, names);
}

const rgb = (hex: string): string => {
  const n = Number.parseInt(hex.slice(1), 16);
  return `rgb(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255})`;
};

const drawn = (target: Locator, property: string): Promise<string> =>
  target.evaluate((el, p) => getComputedStyle(el).getPropertyValue(p), property);

async function openShell(page: Page) {
  await enableViewShell(page);
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  const list = page.getByRole("tabpanel");
  return {
    list,
    title: list.locator('[role="gridcell"][data-col="title"]').first(),
    bpm: list.locator('[role="gridcell"][data-col="bpm"]').first(),
    treeItem: page.getByRole("tree", { name: "Tree" }).getByRole("treeitem").first(),
    tab: page.getByRole("tablist", { name: "Views" }).getByRole("tab", { selected: true }),
  };
}

test("a file overriding one colour changes that role and leaves the others on the Omarchy theme", async ({ page }) => {
  const { treeItem, tab } = await openShell(page);
  await announce(page, { roles: TOKYO_NIGHT, font: FACE });
  const tabLine = await drawn(tab, "border-image-source");
  const ground = await drawn(page.getByTestId("shell-top-bar"), "background-color");

  await announce(page, { roles: TEXT_OVERRIDDEN, font: FACE });
  expect(await drawn(treeItem, "color")).toBe(rgb("#ffcc00"));
  expect(await drawn(tab, "border-image-source")).toBe(tabLine);
  expect(await drawn(page.getByTestId("shell-top-bar"), "background-color")).toBe(ground);
});

test("with no theme file the shell keeps its own face and sizes", async ({ page }) => {
  const { title, bpm, treeItem } = await openShell(page);
  const before = [await drawn(title, "font-size"), await drawn(title, "font-family"), await drawn(bpm, "font-family"), await drawn(treeItem, "font-size")];
  expect(before[0]).toBe("12px");
  expect(before[3]).toBe("12px");
  // The Omarchy theme names the system face for upstream's layout; the
  // shell's own type is not touched by it.
  await announce(page, { roles: TOKYO_NIGHT, font: FACE });
  expect([await drawn(title, "font-size"), await drawn(title, "font-family"), await drawn(bpm, "font-family"), await drawn(treeItem, "font-size")])
    .toEqual(before);
});

test("the file's faces and base sizes apply to the interface and to the numbers", async ({ page }) => {
  const { title, bpm, treeItem, list } = await openShell(page);
  const heading = list.getByRole("columnheader").first();
  await announce(page, {
    roles: TOKYO_NIGHT, font: "DejaVu Sans", interfaceFont: "DejaVu Sans", interfaceFontSize: 14,
    monoFont: "DejaVu Sans Mono", monoFontSize: 13,
  });
  expect(await drawn(title, "font-size")).toBe("14px");
  expect(await drawn(treeItem, "font-size")).toBe("14px");
  // Labels keep their proportion to the base: 10.5 of 12.
  expect(await drawn(heading, "font-size")).toBe("12.25px");
  expect(await drawn(title, "font-family")).toContain("DejaVu Sans");
  expect(await drawn(title, "font-family")).not.toContain("DejaVu Sans Mono");
  expect(await drawn(bpm, "font-family")).toContain("DejaVu Sans Mono");
  expect(await drawn(bpm, "font-size")).toBe("13px");

  // Taking the file away puts the shell's own type back.
  await announce(page, { roles: TOKYO_NIGHT, font: FACE });
  expect(await drawn(title, "font-size")).toBe("12px");
  expect(await drawn(bpm, "font-family")).not.toContain("DejaVu");
});

test("meaning colours are identical with and without a theme file", async ({ page }) => {
  const { list } = await openShell(page);
  const pip = list.getByRole("radiogroup", { name: "Energy 3 of 5" }).first().getByRole("radio").first();
  await expect(pip).toBeVisible();
  const read = async () => [await drawn(pip, "background-color"), ...(await resolved(page, MEANING))];
  const without = await read();
  await announce(page, {
    roles: { ...TOKYO_NIGHT, accent: "#ff0000", onAccent: "#000000" }, font: "DejaVu Sans",
    interfaceFont: "DejaVu Sans", interfaceFontSize: 15,
  });
  expect(await read()).toEqual(without);
});

test("mistakes in the theme file are shown in the status line with their lines, and go when the file is fixed", async ({ page }) => {
  await openShell(page);
  const status = page.getByTestId("status-line").getByRole("status", { name: "Theme file" });
  await expect(status).toHaveCount(0);
  const problems = [
    { line: 3, message: "\"accent\" is not a colour; write it as \"#rrggbb\"" },
    { line: 7, message: "unknown key \"acent\"" },
  ];
  await announce(page, { roles: TOKYO_NIGHT, font: FACE, problems });
  await expect(status).toBeVisible();
  await expect(status).toContainText("line 3");
  await expect(status).toHaveAttribute("title", /line 7: unknown key "acent"/);

  // Put away, it stays away while the same mistakes come with another colour.
  await status.getByRole("button", { name: "Dismiss" }).click();
  await expect(status).toHaveCount(0);
  await announce(page, { roles: TEXT_OVERRIDDEN, font: FACE, problems: problems.map((problem) => ({ ...problem })) });
  await expect(status).toHaveCount(0);
  // Another mistake brings it back.
  await announce(page, { roles: TOKYO_NIGHT, font: FACE, problems: [{ line: 4, message: "unknown key \"y\"" }] });
  await expect(status).toContainText("line 4");
  await announce(page, { roles: TOKYO_NIGHT, font: FACE, problems: [] });
  await expect(status).toHaveCount(0);
});

/**
 * The texts in `cells` that do not fit their box: the rendered text's own
 * box, not a computed size, so a clipped or ellipsised value is caught.
 */
async function overflowing(cells: Locator): Promise<string[]> {
  return cells.evaluateAll((elements) => elements.flatMap((element) => {
    const text = (element.textContent ?? "").trim();
    if (!text) return [];
    const range = document.createRange();
    range.selectNodeContents(element);
    const ink = range.getBoundingClientRect();
    const box = element.getBoundingClientRect();
    const fits = ink.left >= box.left - 1 && ink.right <= box.right + 1 && ink.top >= box.top - 1 && ink.bottom <= box.bottom + 1;
    return fits ? [] : [`${element.getAttribute("data-col") ?? element.getAttribute("role")}: "${text}" ${Math.round(ink.width)}x${Math.round(ink.height)} in ${Math.round(box.width)}x${Math.round(box.height)}`];
  }));
}

for (const size of [12, 16, 24]) {
  test(`at ${size} px the list's values and headings and the tree's rows fit`, async ({ page }) => {
    await page.setViewportSize({ width: 2600, height: 1200 });
    const { list } = await openShell(page);
    await announce(page, {
      roles: TOKYO_NIGHT, font: FACE, interfaceFont: "DejaVu Sans", interfaceFontSize: size,
      monoFont: "DejaVu Sans Mono", monoFontSize: size,
    });
    const numbers = ["trackNo", "bpm", "key", "duration", "bitrate", "dateAdded"]
      .map((col) => `[role="gridcell"][data-col="${col}"]`).join(", ");
    await expect(list.locator('[role="gridcell"][data-col="bpm"]').first()).toBeVisible();
    expect(await overflowing(list.locator(numbers))).toEqual([]);
    expect(await overflowing(list.getByRole("columnheader"))).toEqual([]);
    expect(await overflowing(page.getByRole("tree", { name: "Tree" }).getByRole("treeitem"))).toEqual([]);
    expect(await overflowing(page.getByRole("tablist", { name: "Views" }).getByRole("tab"))).toEqual([]);
  });
}

test("the file's base size applies to the old layout too", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("browser-title")).toContainText("Tracks)", { timeout: 15_000 });
  const cell = page.locator('[role="gridcell"][data-col="title"]').first();
  await expect(cell).toBeVisible();
  const before = Number.parseFloat(await drawn(cell, "font-size"));
  await announce(page, { roles: TOKYO_NIGHT, font: FACE, interfaceFontSize: 18 });
  const after = Number.parseFloat(await drawn(cell, "font-size"));
  expect(after / before).toBeCloseTo(1.5, 2);
  expect(await overflowing(page.locator('[role="gridcell"][data-col="bpm"], [role="gridcell"][data-col="duration"]'))).toEqual([]);
  await announce(page, { roles: TOKYO_NIGHT, font: FACE });
  expect(Number.parseFloat(await drawn(cell, "font-size"))).toBe(before);
});
