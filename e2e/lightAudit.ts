/**
 * Reads what a region of the window is drawn in, the way a DJ would see it:
 * each piece of text, each icon and each input against the ground actually
 * painted behind it (translucent grounds and faded elements composited), and
 * every painted ground. Used by `light-themes.spec.ts` (wbx-45.1).
 */
import type { Locator } from "@playwright/test";

export type Seen = {
  /** What it is, for a failure message: tag, label or text. */
  what: string;
  kind: "text" | "icon" | "placeholder";
  /** The colour drawn, composited over its ground, `rgb(r, g, b)`. */
  colour: string;
  ground: string;
  ratio: number;
  /** Drawn in the secondary text colour (`--role-text-dim`), which needs 3:1, not 4.5:1. */
  secondary: boolean;
  /** Inside a waveform well (`data-wave-well`), which stays dark in every theme. */
  well: boolean;
  /** Its ground is painted by a mark filled with a colour that carries meaning (a phrase, key light, Prep stripe, set cue pad or tag). */
  meaningGround: boolean;
};

export type Painted = { what: string; ground: string; luminance: number; meaning: boolean };

/**
 * Every visible text, icon and empty field's placeholder inside `region`,
 * and every ground painted there. Disabled controls are left out (they need
 * not reach a contrast), as are marks inside `[data-meaning]` grounds, whose
 * colours are fixed, and canvases (waveforms are drawn, not text).
 */
export function audit(region: Locator): Promise<{ seen: Seen[]; painted: Painted[] }> {
  return region.evaluate((root) => {
    type RGBA = [number, number, number, number];
    const parse = (colour: string): RGBA => {
      const parts = (colour.match(/[\d.]+/g) ?? []).map(Number);
      if (colour.startsWith("color(srgb")) return [parts[0]! * 255, parts[1]! * 255, parts[2]! * 255, parts[3] ?? 1];
      return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0, parts[3] ?? 1];
    };
    const over = (top: RGBA, below: RGBA): RGBA => {
      const a = top[3];
      return [top[0] * a + below[0] * (1 - a), top[1] * a + below[1] * (1 - a), top[2] * a + below[2] * (1 - a), 1];
    };
    const css = (c: RGBA) => `rgb(${Math.round(c[0])}, ${Math.round(c[1])}, ${Math.round(c[2])})`;
    const lum = (c: RGBA) => {
      const [r, g, b] = c.slice(0, 3).map((v) => {
        const x = v / 255;
        return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
    };
    const ratio = (a: RGBA, b: RGBA) => {
      const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
      return (hi! + 0.05) / (lo! + 0.05);
    };
    /** The ground painted behind `el`, its own included: translucent layers composited down to the page. */
    const MEANING_FILL = '[data-phrase], [data-light], [data-prep], [data-set], [data-state="set"], [data-meaning-well]';
    const painterOf = (el: Element): Element | null => {
      for (let at: Element | null = el; at; at = at.parentElement) if (parse(getComputedStyle(at).backgroundColor)[3] > 0) return at;
      return null;
    };
    const groundOf = (el: Element | null): RGBA => {
      const layers: RGBA[] = [];
      for (let at = el; at; at = at.parentElement) {
        const c = parse(getComputedStyle(at).backgroundColor);
        if (c[3] > 0) {
          layers.push(c);
          if (c[3] >= 1) break;
        }
      }
      let result: RGBA = layers.length && layers[layers.length - 1]![3] >= 1 ? layers.pop()! : parse(getComputedStyle(document.body).backgroundColor);
      if (result[3] < 1) result = over(result, [255, 255, 255, 1]);
      while (layers.length) result = over(layers.pop()!, result);
      return result;
    };
    const opacityOf = (el: Element): number => {
      let o = 1;
      for (let at: Element | null = el; at; at = at.parentElement) o *= Number(getComputedStyle(at).opacity);
      return o;
    };
    const dim = (() => {
      const probe = document.createElement("i");
      probe.style.color = "var(--role-text-dim)";
      document.body.append(probe);
      const value = parse(getComputedStyle(probe).color);
      probe.remove();
      return css(value);
    })();
    const name = (el: Element): string => {
      const label = el.getAttribute("aria-label") ?? el.closest("[aria-label]")?.getAttribute("aria-label") ?? "";
      const text = (el.textContent ?? "").trim().slice(0, 30);
      const tid = el.closest("[data-testid]")?.getAttribute("data-testid") ?? "";
      return `${el.tagName.toLowerCase()} "${text || label}" in ${tid}`;
    };
    const visible = (el: Element) => el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })
      && el.getClientRects().length > 0 && !el.closest("[inert], [data-kept-hidden]");
    const exempt = (el: Element) => el.closest(":disabled, [aria-disabled='true'], [data-meaning], canvas") !== null;

    const seen: Seen[] = [];
    const painted: Painted[] = [];
    const record = (el: Element, kind: Seen["kind"], drawn: RGBA) => {
      const ground = groundOf(el);
      const colour = over([drawn[0], drawn[1], drawn[2], drawn[3] * opacityOf(el)], ground);
      seen.push({ what: name(el), kind, colour: css(colour), ground: css(ground), ratio: ratio(colour, ground), secondary: css(drawn) === dim, well: el.closest("[data-wave-well]") !== null,
        meaningGround: painterOf(el)?.matches(MEANING_FILL) ?? false });
    };
    for (const el of root.querySelectorAll("*")) {
      if (!visible(el)) continue;
      const style = getComputedStyle(el);
      const bg = parse(style.backgroundColor);
      if (bg[3] > 0.5) {
        painted.push({ what: name(el), ground: css(groundOf(el)), luminance: lum(groundOf(el)), meaning: el.closest("[data-meaning], canvas") !== null });
      }
      if (exempt(el)) continue;
      if (el instanceof SVGSVGElement && !el.parentElement?.closest("svg")) {
        const shape = el.querySelector("path, rect, circle, polygon, line, polyline, use") ?? el;
        const s = getComputedStyle(shape);
        const paint = s.fill !== "none" && !s.fill.startsWith("url") ? s.fill : s.stroke;
        if (paint && paint !== "none") record(el, "icon", parse(paint));
        continue;
      }
      if (el.closest("svg")) continue;
      if ((el instanceof HTMLInputElement && !["checkbox", "radio", "range", "hidden"].includes(el.type)) || el instanceof HTMLTextAreaElement) {
        if (el.value) record(el, "text", parse(style.color));
        else if (el.placeholder) record(el, "placeholder", parse(getComputedStyle(el, "::placeholder").color));
        continue;
      }
      if (el instanceof HTMLSelectElement) { record(el, "text", parse(style.color)); continue; }
      const ownText = [...el.childNodes].some((n) => n.nodeType === Node.TEXT_NODE && (n.textContent ?? "").trim() !== "");
      if (ownText) record(el, "text", parse(style.color));
    }
    return { seen, painted };
  });
}

/** What falls short: text and icons under 4.5:1, secondary text under 3:1, placeholders under 3:1. */
export function shortfalls(seen: Seen[]): string[] {
  return seen
    .filter((s) => s.ratio < (s.secondary || s.kind === "placeholder" ? 3 : 4.5))
    .map((s) => `${s.kind} ${s.what}: ${s.colour} on ${s.ground} = ${s.ratio.toFixed(2)}`);
}
