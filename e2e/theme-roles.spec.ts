/**
 * The chrome follows the Omarchy theme; what the music means does not.
 *
 * The mock interface has no app shell, so it starts on the stylesheet's
 * fallbacks, upstream's own colours and face. The theme's roles are then
 * laid over the root element exactly as the app shell's report would lay
 * them (`themeProperties`), and the real cascade is read back.
 */
import { expect, test, type Page } from "@playwright/test";
import { themeProperties, type Theme, type ThemeRoles } from "../src/lib/themeRoles";

const TOKYO_NIGHT: ThemeRoles = {
  mode: "dark", ground: "#0e0e14", panel: "#13141c", working: "#1a1b26", hairline: "#24283b",
  selection: "#292e42", border: "#414868", text: "#a9b1d6", textDim: "#565f89", textBright: "#c0caf5",
  accent: "#7aa2f7", tabFrom: "#bb9af7", tabTo: "#ff9e64",
};
const GRUVBOX: ThemeRoles = {
  mode: "dark", ground: "#1d2021", panel: "#222526", working: "#282828", hairline: "#3c3836",
  selection: "#504945", border: "#665c54", text: "#d4be98", textDim: "#a89984", textBright: "#ddc7a1",
  accent: "#7daea3", tabFrom: "#d3869b", tabTo: "#d8a657",
};

/** Colours that carry meaning: bands, phrases, cues, Prep colours, meters, the playhead. */
const MEANING = [
  "--c-wave-low", "--c-wave-all", "--c-phrase-chorus", "--c-cue-hot", "--c-cue-purple", "--c-label-pink",
  "--c-comment-red", "--c-meter-green", "--c-playhead", "--c-downbeat", "--c-played",
];
const CHROME = ["--c-black", "--c-top", "--c-panel", "--c-raised", "--c-select", "--c-accent", "--c-text", "--c-text-dim"];

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

async function show(page: Page, theme: Theme): Promise<void> {
  await page.evaluate((properties) => {
    for (const [name, value] of Object.entries(properties)) {
      if (value === null) document.documentElement.style.removeProperty(name);
      else document.documentElement.style.setProperty(name, value);
    }
  }, themeProperties(theme));
}

const rgb = (hex: string): string => {
  const n = Number.parseInt(hex.slice(1), 16);
  return `rgb(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255})`;
};

test("without an app shell the interface keeps upstream's colours and face", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).not.toHaveAttribute("data-theme-mode", /.*/);
  const chrome = await resolved(page, CHROME);
  expect(chrome["--c-black"]).toBe(rgb("#000000"));
  expect(chrome["--c-panel"]).toBe(rgb("#1E1E1E"));
  expect(chrome["--c-select"]).toBe(rgb("#314B67"));
  expect(chrome["--c-accent"]).toBe(rgb("#1373EB"));
  expect(chrome["--c-text"]).toBe(rgb("#E6E6E6"));
  const face = await page.evaluate(() => getComputedStyle(document.body).fontFamily);
  expect(face).toContain("Arial");
});

test("a theme recolours the chrome and leaves every colour that carries meaning alone", async ({ page }) => {
  await page.goto("/");
  const meaningBefore = await resolved(page, MEANING);

  await show(page, { roles: TOKYO_NIGHT, font: "JetBrainsMono Nerd Font" });
  const tokyo = await resolved(page, CHROME);
  expect(tokyo["--c-black"]).toBe(rgb(TOKYO_NIGHT.ground));
  expect(tokyo["--c-top"]).toBe(rgb(TOKYO_NIGHT.panel));
  expect(tokyo["--c-panel"]).toBe(rgb(TOKYO_NIGHT.working));
  expect(tokyo["--c-raised"]).toBe(rgb(TOKYO_NIGHT.hairline));
  expect(tokyo["--c-select"]).toBe(rgb(TOKYO_NIGHT.selection));
  expect(tokyo["--c-accent"]).toBe(rgb(TOKYO_NIGHT.accent));
  expect(tokyo["--c-text"]).toBe(rgb(TOKYO_NIGHT.text));
  expect(tokyo["--c-text-dim"]).toBe(rgb(TOKYO_NIGHT.textDim));
  expect(await page.evaluate(() => getComputedStyle(document.body).fontFamily)).toContain("JetBrainsMono Nerd Font");
  expect(await resolved(page, MEANING)).toEqual(meaningBefore);

  // A second theme, without a reload: the switch a DJ makes while the app is open.
  await show(page, { roles: GRUVBOX, font: "JetBrainsMono Nerd Font" });
  const gruvbox = await resolved(page, CHROME);
  expect(gruvbox["--c-panel"]).toBe(rgb(GRUVBOX.working));
  expect(gruvbox["--c-accent"]).toBe(rgb(GRUVBOX.accent));
  expect(await resolved(page, MEANING)).toEqual(meaningBefore);

  // The theme goes away: back to upstream's whole.
  await show(page, { roles: null, font: null });
  expect((await resolved(page, CHROME))["--c-accent"]).toBe(rgb("#1373EB"));
});
