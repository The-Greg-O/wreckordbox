import { describe, expect, it } from "vitest";
import { contrastRatio, legibleRoles } from "./legibleRoles";
import type { ThemeRoles } from "./themeRoles";

/** Omarchy's stock palettes, as the app shell reports them (`/usr/share/omarchy/themes/<slug>/colors.toml`). */
const LATTE: ThemeRoles = {
  mode: "light", ground: "#d7d8dc", panel: "#e3e4e8", working: "#eff1f5", hairline: "#dce0e8",
  selection: "#ccd0da", border: "#acb0be", text: "#4c4f69", textDim: "#9ca0b0", textBright: "#4c4f69",
  accent: "#1e66f5", onAccent: "#ffffff", tabFrom: "#ea76cb", tabTo: "#df8e1d",
};
const WHITE: ThemeRoles = {
  mode: "light", ground: "#e8e8e8", panel: "#f5f5f5", working: "#ffffff", hairline: "#c0c0c0",
  selection: "#c0c0c0", border: "#808080", text: "#000000", textDim: "#c0c0c0", textBright: "#000000",
  accent: "#6e6e6e", onAccent: "#ffffff", tabFrom: "#2e2e2e", tabTo: "#4a4a4a",
};
const LUPINE: ThemeRoles = {
  mode: "light", ground: "#dedede", panel: "#ececec", working: "#fafafa", hairline: "#f5f5f5",
  selection: "#d0d0d0", border: "#9e9e9e", text: "#212121", textDim: "#757575", textBright: "#000000",
  accent: "#3264eb", onAccent: "#ffffff", tabFrom: "#b363ff", tabTo: "#358fff",
};
const TOKYO_NIGHT: ThemeRoles = {
  mode: "dark", ground: "#0e0e14", panel: "#13141c", working: "#1a1b26", hairline: "#24283b",
  selection: "#292e42", border: "#414868", text: "#a9b1d6", textDim: "#565f89", textBright: "#c0caf5",
  accent: "#7aa2f7", onAccent: "#0e0e14", tabFrom: "#bb9af7", tabTo: "#ff9e64",
};

const grounds = (roles: ThemeRoles) => [roles.ground, roles.panel, roles.working, roles.hairline, roles.selection];
const worst = (colour: string, roles: ThemeRoles) => Math.min(...grounds(roles).map((g) => contrastRatio(colour, g)));

describe("contrastRatio", () => {
  it("is WCAG's ratio", () => {
    expect(contrastRatio("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrastRatio("#ffffff", "#ffffff")).toBeCloseTo(1, 5);
    // A worked example: #767676 on white is the classic 4.54:1.
    expect(contrastRatio("#767676", "#ffffff")).toBeCloseTo(4.54, 2);
  });
});

describe("legibleRoles under a light theme", () => {
  it("lifts secondary text that a light theme draws too faint to at least 3:1 on every ground", () => {
    // Latte's dark_foreground is about 2:1 on its own grounds; White's 1.8:1.
    expect(worst(LATTE.textDim, LATTE)).toBeLessThan(3);
    expect(worst(WHITE.textDim, WHITE)).toBeLessThan(3);
    for (const roles of [LATTE, WHITE]) {
      const lifted = legibleRoles(roles);
      expect(worst(lifted.textDim, roles), roles.textDim).toBeGreaterThanOrEqual(3);
      // Still secondary: no darker than the text itself.
      expect(contrastRatio(lifted.textDim, roles.working)).toBeLessThan(contrastRatio(roles.text, roles.working));
    }
  });

  it("lifts text and emphasised text to at least 4.5:1 on every ground", () => {
    const pale: ThemeRoles = { ...LATTE, text: "#8c8fa1", textBright: "#9ca0b0" };
    const lifted = legibleRoles(pale);
    expect(worst(lifted.text, pale)).toBeGreaterThanOrEqual(4.5);
    expect(worst(lifted.textBright, pale)).toBeGreaterThanOrEqual(4.5);
  });

  it("lifts an accent that would not stand out as a glyph or focus ring to at least 3:1", () => {
    expect(worst(WHITE.accent, WHITE)).toBeLessThan(4.5);
    const pale: ThemeRoles = { ...LATTE, accent: "#7da6ff" };
    expect(worst(pale.accent, pale)).toBeLessThan(3);
    expect(worst(legibleRoles(pale).accent, pale)).toBeGreaterThanOrEqual(3);
  });

  it("leaves a role the theme already draws readably as the theme gives it, and the grounds untouched", () => {
    // Lupine's ink reaches its contrast everywhere but its dim text on a selected row (2.99:1).
    const lifted = legibleRoles(LUPINE);
    expect({ ...lifted, textDim: LUPINE.textDim }).toEqual(LUPINE);
    expect(lifted.textDim).not.toBe(LUPINE.textDim);
    const latte = legibleRoles(LATTE);
    expect(latte.text).toBe(LATTE.text);
    expect(latte.accent).toBe(LATTE.accent);
    expect(grounds(latte)).toEqual(grounds(LATTE));
    expect([latte.border, latte.onAccent, latte.tabFrom, latte.tabTo]).toEqual([LATTE.border, LATTE.onAccent, LATTE.tabFrom, LATTE.tabTo]);
  });

  it("leaves a value that is not a colour for the stylesheet's fallback", () => {
    const broken = legibleRoles({ ...WHITE, textDim: "grey" });
    expect(broken.textDim).toBe("grey");
    expect(legibleRoles({ ...WHITE, working: "" })).toEqual({ ...WHITE, working: "" });
  });

  it("does not touch a dark theme", () => {
    expect(legibleRoles(TOKYO_NIGHT)).toBe(TOKYO_NIGHT);
  });
});
