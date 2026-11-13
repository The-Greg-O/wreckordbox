/**
 * A light theme's ink, made readable on its own grounds (wbx-45.1).
 *
 * Light Omarchy palettes often give a secondary text colour that is too
 * faint to read on their grounds (Catppuccin Latte's `dark_foreground` is
 * about 2:1, White's 1.8:1). Under a light theme each ink role that falls
 * short is moved towards black, a step at a time, until it reaches its
 * contrast on every ground the view shell draws it on: text and emphasised
 * text 4.5:1, secondary text 3:1, the accent (focus rings, lit glyphs) 3:1.
 * A role that already reaches it is the theme's own; grounds, borders and
 * the tab line are never changed, and the on-accent colour only when a
 * lifted accent no longer carries it at 4.5:1. Dark themes are
 * returned as they are.
 */

import type { ThemeRoles } from "./themeRoles";

const channels = (hex: string): number[] => [1, 3, 5].map((at) => Number.parseInt(hex.slice(at, at + 2), 16));

function luminance(hex: string): number {
  const [r, g, b] = channels(hex).map((value) => {
    const c = value / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
}

/** WCAG 2 contrast ratio of two `#rrggbb` colours, 1 to 21. */
export function contrastRatio(one: string, other: string): number {
  const [high, low] = [luminance(one), luminance(other)].sort((a, b) => b - a);
  return (high! + 0.05) / (low! + 0.05);
}

const isColour = (value: unknown): value is string => typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value);

const hex = (rgb: number[]): string => `#${rgb.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")}`;

/** `colour` moved towards black until it reaches `target` on each of `grounds`. */
function darkened(colour: string, grounds: string[], target: number): string {
  const reaches = (c: string) => grounds.every((ground) => contrastRatio(c, ground) >= target);
  if (reaches(colour)) return colour;
  const start = channels(colour);
  for (let step = 1; step <= 20; step++) {
    const candidate = hex(start.map((value) => value * (1 - step * 0.05)));
    if (reaches(candidate)) return candidate;
  }
  return "#000000";
}

/** The minimum contrast each ink role must reach on every ground. */
const TARGETS = { text: 4.5, textBright: 4.5, textDim: 3, accent: 3 } as const;

export function legibleRoles(roles: ThemeRoles): ThemeRoles {
  if (roles.mode !== "light") return roles;
  const grounds = [roles.ground, roles.panel, roles.working, roles.hairline, roles.selection];
  // A value that is not a colour is left for the stylesheet's fallback to stand in for.
  if (!grounds.every(isColour)) return roles;
  const lifted = { ...roles };
  let changed = false;
  for (const [role, target] of Object.entries(TARGETS) as [keyof typeof TARGETS, number][]) {
    if (!isColour(roles[role])) continue;
    const value = darkened(roles[role], grounds, target);
    if (value !== roles[role]) {
      lifted[role] = value;
      changed = true;
    }
  }
  // A darker accent may no longer carry the label the theme picked for it.
  if (lifted.accent !== roles.accent && isColour(lifted.onAccent) && contrastRatio(lifted.onAccent, lifted.accent) < 4.5) {
    lifted.onAccent = labelOn(lifted.accent);
  }
  return changed ? lifted : roles;
}

/** The better of black and white on `fill`. */
const labelOn = (fill: string): string => (contrastRatio("#ffffff", fill) >= contrastRatio("#000000", fill) ? "#ffffff" : "#000000");

/**
 * The accent the view shell draws text, glyphs and fills in under a light
 * theme: the theme's accent moved towards black until it reads at 4.5:1 on
 * every ground (a focus ring or border needs only the 3:1 of `legibleRoles`),
 * and the label written on it (the theme's on-accent colour while that
 * still reads at 4.5:1, else black or white). Null under a dark theme.
 */
export function accentInk(roles: ThemeRoles): { ink: string; onInk: string } | null {
  const ink = writtenIn(roles.accent, roles);
  if (ink === null) return null;
  const onInk = isColour(roles.onAccent) && contrastRatio(roles.onAccent, ink) >= 4.5 ? roles.onAccent : labelOn(ink);
  return { ink, onInk };
}

/**
 * `colour` (one of a light theme's own, such as the tab line's end) as the
 * view shell writes text in it: moved towards black until it reads at 4.5:1
 * on every ground. Null under a dark theme, or when it is not a colour.
 */
export function writtenIn(colour: string, roles: ThemeRoles): string | null {
  if (roles.mode !== "light") return null;
  const grounds = [roles.ground, roles.panel, roles.working, roles.hairline, roles.selection];
  if (!grounds.every(isColour) || !isColour(colour)) return null;
  return darkened(colour, grounds, 4.5);
}
