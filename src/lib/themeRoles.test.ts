import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { applyTheme, themeProperties, type Theme, type ThemeRoles } from "./themeRoles";

const TOKYO_NIGHT: ThemeRoles = {
  mode: "dark", ground: "#0e0e14", panel: "#13141c", working: "#1a1b26", hairline: "#24283b",
  selection: "#292e42", border: "#414868", text: "#a9b1d6", textDim: "#565f89", textBright: "#c0caf5",
  accent: "#7aa2f7", tabFrom: "#bb9af7", tabTo: "#ff9e64",
};
const GRUVBOX: ThemeRoles = {
  mode: "dark", ground: "#161616", panel: "#1e1e1e", working: "#282828", hairline: "#3c3836",
  selection: "#504945", border: "#665c54", text: "#d4be98", textDim: "#7c6f64", textBright: "#d4be98",
  accent: "#7daea3", tabFrom: "#d3869b", tabTo: "#d8a657",
};

const LATTE: ThemeRoles = {
  mode: "light", ground: "#d7d8dc", panel: "#e3e4e8", working: "#eff1f5", hairline: "#dce0e8",
  selection: "#ccd0da", border: "#acb0be", text: "#4c4f69", textDim: "#9ca0b0", textBright: "#4c4f69",
  accent: "#1e66f5", tabFrom: "#8839ef", tabTo: "#df8e1d",
};

/** A stand-in for the root element. */
function fakeRoot() {
  const set = new Map<string, string>();
  const attributes = new Map<string, string>();
  return {
    set,
    attributes,
    style: {
      setProperty: (name: string, value: string) => void set.set(name, value),
      removeProperty: (name: string) => void set.delete(name),
    },
    setAttribute: (name: string, value: string) => void attributes.set(name, value),
    removeAttribute: (name: string) => void attributes.delete(name),
  };
}

describe("themeProperties", () => {
  it("names each role's custom property", () => {
    expect(themeProperties({ roles: TOKYO_NIGHT, font: "JetBrainsMono Nerd Font" })).toEqual({
      "--role-ground": "#0e0e14",
      "--role-panel": "#13141c",
      "--role-working": "#1a1b26",
      "--role-hairline": "#24283b",
      "--role-control": "#24283b",
      "--role-selection": "#292e42",
      "--role-border": "#414868",
      "--role-text": "#a9b1d6",
      "--role-text-dim": "#565f89",
      "--role-text-bright": "#c0caf5",
      "--role-accent": "#7aa2f7",
      "--role-tab-from": "#bb9af7",
      "--role-tab-to": "#ff9e64",
      "--role-font": '"JetBrainsMono Nerd Font", ui-monospace, monospace',
    });
  });

  it("leaves every role to the stylesheet's fallback when there is no theme and no font", () => {
    const properties = themeProperties({ roles: null, font: null });
    expect(Object.keys(properties)).toHaveLength(14);
    expect(Object.values(properties).every((value) => value === null)).toBe(true);
  });

  it("passes over a value that is not a colour, and a font name that could not be quoted", () => {
    const properties = themeProperties({
      roles: { ...TOKYO_NIGHT, accent: "red; } body { display: none" },
      font: 'Mono"; } body { display: none',
    });
    expect(properties["--role-accent"]).toBeNull();
    expect(properties["--role-font"]).toBeNull();
    expect(properties["--role-text"]).toBe("#a9b1d6");
  });
});

describe("applyTheme", () => {
  it("sets the roles, and takes them away again when the theme goes", () => {
    const root = fakeRoot();
    applyTheme({ roles: TOKYO_NIGHT, font: null }, root);
    expect(root.set.get("--role-accent")).toBe("#7aa2f7");
    expect(root.set.has("--role-font")).toBe(false);
    applyTheme({ roles: GRUVBOX, font: null }, root);
    expect(root.set.get("--role-accent")).toBe("#7daea3");
    applyTheme({ roles: null, font: null }, root);
    expect(root.set.size).toBe(0);
  });

  it("marks the root with the theme's mode, and unmarks it when the theme goes", () => {
    const root = fakeRoot();
    applyTheme({ roles: LATTE, font: null }, root);
    expect(root.attributes.get("data-theme-mode")).toBe("light");
    applyTheme({ roles: TOKYO_NIGHT, font: null }, root);
    expect(root.attributes.get("data-theme-mode")).toBe("dark");
    applyTheme({ roles: null, font: null }, root);
    expect(root.attributes.has("data-theme-mode")).toBe(false);
  });
});

// What the window would compute for each token under a theme: the
// stylesheet's `:root` declarations with the theme's properties laid over
// them, as an inline style on the root lays them over.
const tokensCss = readFileSync(new URL("../styles/tokens.css", import.meta.url), "utf8");

/** The declarations of every rule whose selector is one of `selectors`, later rules winning. */
function declared(selectors = [":root"]): Map<string, string> {
  const found = new Map<string, string>();
  for (const [, selector, body] of tokensCss.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (!selectors.includes(selector!.trim())) continue;
    for (const [, name, value] of body!.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) found.set(name!, value!.trim());
  }
  return found;
}

function computed(theme: Theme): Map<string, string> {
  const values = declared(theme.roles ? [":root", `:root[data-theme-mode="${theme.roles.mode}"]`] : [":root"]);
  for (const [name, value] of Object.entries(themeProperties(theme))) if (value !== null) values.set(name, value);
  const resolve = (value: string, depth = 0): string =>
    depth > 8 ? value : value.replace(/var\((--[\w-]+)\)/g, (_, name: string) => resolve(values.get(name) ?? "", depth + 1));
  return new Map([...values].map(([name, value]) => [name, resolve(value)]));
}

/** The tokens `docs/design/tokens.md` replaces with roles. */
const CHROME = [
  "--c-black", "--c-top", "--c-panel", "--c-raised", "--c-button", "--c-text", "--c-text-dim", "--c-accent",
  "--c-select", "--f-ui",
];

/** Colours that mean something: the music, not the chrome. */
const MEANING = /^--c-(wave|phrase|cue|pad|label|comment|meter|playhead|beat|downbeat|vocal|pcm|scrub|played|missing$|error|tempo|traffic-light|play-ring|bars-label|sync-pending)/;

describe("tokens under a theme", () => {
  const plain = computed({ roles: null, font: null });
  const tokyo = computed({ roles: TOKYO_NIGHT, font: "JetBrainsMono Nerd Font" });
  const gruvbox = computed({ roles: GRUVBOX, font: "CaskaydiaMono Nerd Font" });

  it("only the chrome tokens refer to a role", () => {
    const referring = [...declared()].filter(([name, value]) => !name.startsWith("--role-") && value.includes("--role-"));
    expect(referring.map(([name]) => name).sort()).toEqual([...CHROME].sort());
  });

  it("colours that carry meaning are identical under two themes and under none", () => {
    const meaning = [...plain.keys()].filter((name) => MEANING.test(name));
    // Waveform bands, phrases, hot cue and pad colours, Prep colours, meters.
    expect(meaning.length).toBeGreaterThan(80);
    expect(meaning).toEqual(expect.arrayContaining([
      "--c-wave-low", "--c-wave-all", "--c-phrase-chorus", "--c-cue-hot", "--c-cue-purple", "--c-label-pink",
      "--c-comment-red", "--c-meter-green", "--c-playhead", "--c-downbeat",
    ]));
    for (const name of meaning) {
      expect(tokyo.get(name), name).toBe(plain.get(name));
      expect(gruvbox.get(name), name).toBe(plain.get(name));
      expect(plain.get(name), name).not.toContain("var(");
    }
    expect(plain.get("--c-wave-low")).toBe("#0055E1");
    expect(plain.get("--c-phrase-chorus")).toBe("#4EA730");
  });

  it("nothing but the chrome tokens and the roles changes with the theme", () => {
    const changed = [...plain.keys()].filter((name) => tokyo.get(name) !== plain.get(name));
    expect(changed.filter((name) => !name.startsWith("--role-")).sort()).toEqual([...CHROME].sort());
  });

  it("the chrome takes the theme's colours and the system's face", () => {
    expect(tokyo.get("--c-black")).toBe("#0e0e14");
    expect(tokyo.get("--c-panel")).toBe("#1a1b26");
    expect(tokyo.get("--c-select")).toBe("#292e42");
    expect(tokyo.get("--c-accent")).toBe("#7aa2f7");
    expect(tokyo.get("--c-text")).toBe("#a9b1d6");
    expect(gruvbox.get("--c-accent")).toBe("#7daea3");
    expect(tokyo.get("--f-ui")).toBe('"JetBrainsMono Nerd Font", ui-monospace, monospace');
  });

  it("under a light theme the upstream layout keeps its dark grounds and light text, and takes the accent", () => {
    // Upstream's layout draws white icons and labels from dozens of fixed
    // tokens; on a light ground they vanish. The roles themselves are light,
    // for the view shell to use.
    const latte = computed({ roles: LATTE, font: null });
    expect(latte.get("--role-working")).toBe("#eff1f5");
    expect(latte.get("--role-text")).toBe("#4c4f69");
    for (const name of ["--c-black", "--c-top", "--c-panel", "--c-raised", "--c-button", "--c-select", "--c-text", "--c-text-dim"]) {
      expect(latte.get(name), name).toBe(plain.get(name));
    }
    expect(latte.get("--c-accent")).toBe("#1e66f5");
    const meaning = [...plain.keys()].filter((name) => MEANING.test(name));
    for (const name of meaning) expect(latte.get(name), name).toBe(plain.get(name));
  });

  it("without a theme the chrome keeps upstream's colours and face", () => {
    expect(plain.get("--c-black")).toBe("#000000");
    expect(plain.get("--c-top")).toBe("#0F0F0F");
    expect(plain.get("--c-panel")).toBe("#1E1E1E");
    expect(plain.get("--c-raised")).toBe("#323232");
    expect(plain.get("--c-button")).toBe("#2B2B2B");
    expect(plain.get("--c-select")).toBe("#314B67");
    expect(plain.get("--c-accent")).toBe("#1373EB");
    expect(plain.get("--c-text")).toBe("#E6E6E6");
    expect(plain.get("--c-text-dim")).toBe("#9A9A9A");
    expect(plain.get("--f-ui")).toBe('Arial, "Helvetica Neue", Helvetica, sans-serif');
  });
});
