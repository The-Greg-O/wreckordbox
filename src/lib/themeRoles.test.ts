import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { applyTheme, followThemeFrom, themeProperties, type Theme, type ThemeRoles } from "./themeRoles";

const TOKYO_NIGHT: ThemeRoles = {
  mode: "dark", ground: "#0e0e14", panel: "#13141c", working: "#1a1b26", hairline: "#24283b",
  selection: "#292e42", border: "#414868", text: "#a9b1d6", textDim: "#565f89", textBright: "#c0caf5",
  accent: "#7aa2f7", onAccent: "#0e0e14", tabFrom: "#bb9af7", tabTo: "#ff9e64",
};
const GRUVBOX: ThemeRoles = {
  mode: "dark", ground: "#161616", panel: "#1e1e1e", working: "#282828", hairline: "#3c3836",
  selection: "#504945", border: "#665c54", text: "#d4be98", textDim: "#7c6f64", textBright: "#d4be98",
  accent: "#7daea3", onAccent: "#161616", tabFrom: "#d3869b", tabTo: "#d8a657",
};

const LATTE: ThemeRoles = {
  mode: "light", ground: "#d7d8dc", panel: "#e3e4e8", working: "#eff1f5", hairline: "#dce0e8",
  selection: "#ccd0da", border: "#acb0be", text: "#4c4f69", textDim: "#9ca0b0", textBright: "#4c4f69",
  accent: "#1e66f5", onAccent: "#ffffff", tabFrom: "#8839ef", tabTo: "#df8e1d",
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
      "--role-on-accent": "#0e0e14",
      "--role-accent-hover": "#8eb0f8",
      "--role-tab-from": "#bb9af7",
      "--role-tab-to": "#ff9e64",
      "--role-accent-ink": null,
      "--role-on-accent-ink": null,
      "--role-tab-to-ink": null,
      "--role-font": '"JetBrainsMono Nerd Font", ui-monospace, monospace',
      "--role-font-interface": null,
      "--role-font-size": null,
      "--role-font-mono": null,
      "--role-font-mono-size": null,
      "--role-font-scale": null,
      "--role-shell-tree-w": null,
      "--role-shell-row-h": null,
      "--role-shell-heading-h": null,
      "--role-shell-status-h": null,
      "--role-shell-top-bar-h": null,
    });
  });

  it("sets the theme file's faces and sizes, and only those it gives", () => {
    const properties = themeProperties({
      roles: null, font: "Inter", interfaceFont: "Inter", interfaceFontSize: 14, monoFont: "Iosevka", monoFontSize: 11.5,
    });
    expect(properties["--role-font"]).toBe('"Inter", ui-monospace, monospace');
    expect(properties["--role-font-interface"]).toBe('"Inter", ui-monospace, monospace');
    expect(properties["--role-font-size"]).toBe("14px");
    expect(properties["--role-font-mono"]).toBe('"Iosevka", ui-monospace, monospace');
    expect(properties["--role-font-mono-size"]).toBe("11.5px");
    expect(properties["--role-font-scale"]).toBe(String(14 / 12));
    expect(properties["--role-shell-row-h"]).toBe("28px");
    const sizeOnly = themeProperties({ roles: null, font: null, interfaceFontSize: 13 });
    expect(sizeOnly["--role-font-size"]).toBe("13px");
    expect(sizeOnly["--role-font-interface"]).toBeNull();
    expect(sizeOnly["--role-font-mono-size"]).toBeNull();
  });

  it("passes over a size that is not a sensible number and a face that could not be quoted", () => {
    for (const size of [Number.NaN, 0, 3, 200, Number.POSITIVE_INFINITY]) {
      expect(themeProperties({ roles: null, font: null, interfaceFontSize: size, monoFontSize: size })["--role-font-size"], String(size)).toBeNull();
    }
    expect(themeProperties({ roles: null, font: null, monoFont: 'Mono"; } body { display: none' })["--role-font-mono"]).toBeNull();
  });

  it("under a light theme gives the view shell readable ink: faint secondary text lifted, and the accent and tab line's end to write with", () => {
    const properties = themeProperties({ roles: LATTE, font: null });
    // Latte's dark_foreground (#9ca0b0) is about 2:1 on its grounds.
    expect(properties["--role-text-dim"]).not.toBe(LATTE.textDim);
    expect(properties["--role-text"]).toBe(LATTE.text);
    expect(properties["--role-accent"]).toBe(LATTE.accent);
    expect(properties["--role-working"]).toBe(LATTE.working);
    expect(properties["--role-accent-ink"]).toMatch(/^#[0-9a-f]{6}$/);
    expect(properties["--role-accent-ink"]).not.toBe(LATTE.accent);
    expect(properties["--role-on-accent-ink"]).toBe(LATTE.onAccent);
    expect(properties["--role-tab-to-ink"]).not.toBe(LATTE.tabTo);
  });

  it("leaves every role to the stylesheet's fallback when there is no theme and no font", () => {
    const properties = themeProperties({ roles: null, font: null });
    expect(Object.keys(properties)).toHaveLength(29);
    expect(Object.values(properties).every((value) => value === null)).toBe(true);
  });

  it("a lit control under the pointer moves its accent away from what is written on it", () => {
    // A dark label on a light accent: lighter. A white label on a deep accent: darker.
    expect(themeProperties({ roles: TOKYO_NIGHT, font: null })["--role-accent-hover"]).toBe("#8eb0f8");
    expect(themeProperties({ roles: LATTE, font: null })["--role-accent-hover"]).toBe("#1a57d0");
  });

  it("passes over a value that is not a colour, and a font name that could not be quoted", () => {
    const properties = themeProperties({
      roles: { ...TOKYO_NIGHT, accent: "red; } body { display: none" },
      font: 'Mono"; } body { display: none',
    });
    expect(properties["--role-accent"]).toBeNull();
    expect(properties["--role-accent-hover"]).toBeNull();
    expect(properties["--role-font"]).toBeNull();
    expect(properties["--role-text"]).toBe("#a9b1d6");
  });
});

describe("the built-in look under a theme file", () => {
  const BUILT_IN: ThemeRoles = {
    mode: "dark", ground: "#000000", panel: "#0f0f0f", working: "#1e1e1e", hairline: "#323232",
    selection: "#314b67", border: "#ff00ff", text: "#e6e6e6", textDim: "#9a9a9a", textBright: "#ffffff",
    accent: "#1373eb", onAccent: "#f0f0f0", tabFrom: "#1373eb", tabTo: "#1373eb",
    control: "#2b2b2b", accentHover: "#4a82ed", builtIn: true,
  };

  it("keeps the built-in control fill and lit hover the app shell names", () => {
    const properties = themeProperties({ roles: BUILT_IN, font: null });
    expect(properties["--role-control"]).toBe("#2b2b2b");
    expect(properties["--role-accent-hover"]).toBe("#4a82ed");
    expect(properties["--role-border"]).toBe("#ff00ff");
  });

  it("does not switch upstream's lit chrome to the accent, as a theme does", () => {
    const root = fakeRoot();
    applyTheme({ roles: BUILT_IN, font: null }, root);
    expect(root.attributes.has("data-theme-mode")).toBe(false);
    applyTheme({ roles: { ...BUILT_IN, mode: "light" }, font: null }, root);
    expect(root.attributes.get("data-theme-mode")).toBe("light");
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

describe("followThemeFrom", () => {
  /** A stand-in for the app shell: the command's answer, and the event's listeners. */
  function shell(first: Theme) {
    const listeners: ((theme: Theme) => void)[] = [];
    const calls: string[] = [];
    return {
      calls,
      emit: (theme: Theme) => listeners.forEach((listener) => listener(theme)),
      current: () => (calls.push("current"), Promise.resolve(first)),
      onChange: (listener: (theme: Theme) => void) => (calls.push("onChange"), listeners.push(listener), Promise.resolve()),
    };
  }

  it("shows the shell's theme, then each one it announces, with the mode", async () => {
    const root = fakeRoot();
    const source = shell({ roles: TOKYO_NIGHT, font: "JetBrainsMono Nerd Font" });
    await followThemeFrom(source, root);
    expect(root.set.get("--role-accent")).toBe("#7aa2f7");
    expect(root.set.get("--role-on-accent")).toBe("#0e0e14");
    expect(root.set.get("--role-font")).toContain("JetBrainsMono Nerd Font");
    expect(root.attributes.get("data-theme-mode")).toBe("dark");

    source.emit({ roles: LATTE, font: "JetBrainsMono Nerd Font" });
    expect(root.set.get("--role-accent")).toBe("#1e66f5");
    expect(root.attributes.get("data-theme-mode")).toBe("light");

    source.emit({ roles: null, font: null });
    expect(root.set.size).toBe(0);
    expect(root.attributes.has("data-theme-mode")).toBe(false);
  });

  it("subscribes before it asks, so a switch in between is not lost", async () => {
    const source = shell({ roles: null, font: null });
    await followThemeFrom(source, fakeRoot());
    expect(source.calls).toEqual(["onChange", "current"]);
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
  const values = declared(
    theme.roles ? [":root", ":root[data-theme-mode]", `:root[data-theme-mode="${theme.roles.mode}"]`] : [":root"],
  );
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

/** The lit states of upstream's chrome and what is written on them: the accent, under a theme only. */
const LIT = [
  "--c-on-accent", "--c-pad-active", "--c-tab-active", "--c-filter-accent", "--c-mytag-check-on", "--c-mytag-tick",
  "--c-device-tab-on", "--c-info-tab-active", "--c-info-tab-active-text", "--c-rail-lit", "--c-pref-select",
  "--c-lit-hover", "--c-pad-edit-on", "--c-pad-edit-on-hover", "--c-mytag-check-on-off",
];

/** Colours that mean something: the music, not the chrome. */
const MEANING = /^--c-(?!pad-active$|pad-edit-on$|pad-edit-on-hover$)(wave|phrase|cue|pad|energy|loop-band|label|comment|meter|playhead|beat|downbeat|vocal|pcm|scrub|played|missing$|error|tempo|traffic-light|play-ring|bars-label|sync-pending)/;

describe("tokens under a theme", () => {
  const plain = computed({ roles: null, font: null });
  const tokyo = computed({ roles: TOKYO_NIGHT, font: "JetBrainsMono Nerd Font" });
  const gruvbox = computed({ roles: GRUVBOX, font: "CaskaydiaMono Nerd Font" });

  it("only the chrome tokens refer to a role", () => {
    const referring = (selectors: string[]) =>
      [...declared(selectors)].filter(([name, value]) => !name.startsWith("--role-") && value.includes("--role-")).map(([name]) => name).sort();
    // The type sizes follow the theme file's font_size, not a colour.
    const sizes = [...declared([":root"])].filter(([name]) => name.startsWith("--f-size-")).map(([name]) => name);
    expect(sizes.length).toBeGreaterThan(30);
    const scaled = referring([":root"]).filter((name) => name.startsWith("--f-size-"));
    expect(scaled).toEqual(sizes.filter((name) => name !== "--f-size-preview-cue").sort());
    expect(referring([":root"]).filter((name) => !name.startsWith("--f-size-"))).toEqual([...CHROME].sort());
    expect(referring([":root[data-theme-mode]"])).toEqual([...LIT].sort());
    expect([...CHROME, ...LIT].filter((name) => MEANING.test(name))).toEqual([]);
  });

  it("colours that carry meaning are identical under two themes and under none", () => {
    const meaning = [...plain.keys()].filter((name) => MEANING.test(name));
    // Waveform bands, phrases, hot cue and pad colours, Prep colours, meters.
    expect(meaning.length).toBeGreaterThan(80);
    expect(meaning).toEqual(expect.arrayContaining([
      "--c-wave-low", "--c-wave-all", "--c-phrase-chorus", "--c-cue-hot", "--c-cue-purple", "--c-label-pink",
      "--c-comment-red", "--c-meter-green", "--c-playhead", "--c-downbeat", "--c-energy-0", "--c-energy-1",
      "--c-energy-5", "--c-loop-band",
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
    const changed = [...new Set([...plain.keys(), ...tokyo.keys()])].filter((name) => tokyo.get(name) !== plain.get(name));
    expect(changed.filter((name) => !name.startsWith("--role-")).sort()).toEqual([...CHROME, ...LIT].sort());
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

  it("the lit states of the chrome are the accent, and what is written on them the theme's colour for that", () => {
    for (const name of LIT.filter((name) => !/on-accent|tick|text|hover|on-off/.test(name))) {
      expect(tokyo.get(name), name).toBe("#7aa2f7");
      expect(gruvbox.get(name), name).toBe("#7daea3");
    }
    for (const name of ["--c-on-accent", "--c-mytag-tick", "--c-info-tab-active-text"]) expect(tokyo.get(name), name).toBe("#0e0e14");
    // Without a theme they are upstream's measured blues, and nothing defines --c-on-accent.
    expect(plain.get("--c-pad-active")).toBe("#3571E3");
    expect(plain.get("--c-tab-active")).toBe("#457FE5");
    expect(plain.get("--c-pref-select")).toBe("#3571E3");
    expect(plain.get("--c-info-tab-active-text")).toBe("#000000");
    expect(plain.has("--c-on-accent")).toBe(false);
    // A lit control under the pointer, and a ticked box greyed.
    expect(tokyo.get("--c-lit-hover")).toBe("#8eb0f8");
    expect(plain.get("--c-lit-hover")).toBe("#4A82ED");
    expect(plain.get("--c-mytag-check-on-off")).toBe("#144480");
    expect(tokyo.get("--c-mytag-check-on-off")).toBe("color-mix(in srgb, #7aa2f7 50%, #282828)");
  });

  it("Energy stars and the loop band keep their colours under every theme", () => {
    for (const theme of [plain, tokyo, gruvbox]) {
      // One set of Energy colours for upstream's stars and the new list's pips.
      expect(["0", "1", "2", "3", "4", "5"].map((level) => theme.get(`--c-energy-${level}`))).toEqual(
        ["#414868", "#7aa2f7", "#0db9d7", "#9ece6a", "#ff9e64", "#f7768e"],
      );
      expect(theme.has("--c-energy")).toBe(false);
      expect(theme.has("--c-energy-hover")).toBe(false);
      expect(theme.get("--c-loop-band")).toBe("#3571E3");
    }
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
