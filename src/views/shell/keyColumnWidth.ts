/**
 * The Library list's Key column is never narrower than a key needs (wbx-45.10).
 *
 * A Key cell holds one key in the notation on show, and, when the loaded
 * track's key goes with it, a badge with a bold `=` or `↑` (LibraryKey). Its
 * stored width is for 12 px type and grows with the theme file's number size,
 * but the cell's padding does not, and the face can be any the theme file
 * names; so at small sizes or in a wide face the widest lit key ran into the
 * padding. The floor is measured from the face and size the cells are drawn
 * in, over every key in both notations, so a notation switch does not move
 * the column.
 */
import { fromCamelot } from "@/lib/camelot";

/** Every key the wheel knows, as each notation shows it. */
const CODES = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].flatMap((n) => [`${n}A`, `${n}B`]);
const KEY_TEXTS: readonly string[] = [...CODES, ...CODES.map((code) => fromCamelot(code))];
const MARKS = ["=", "↑"] as const;

/** LibraryKey.module.css: the badge's gap and its padding either side. */
const BADGE_GAP = 2;
const BADGE_PADDING = 2 * 2;
/** TrackTable.module.css `.cell`: 6 px padding either side and the 1 px rule. */
const CELL_EDGES = 6 * 2 + 1;

/** `measure(text, bold)`: the drawn width of `text` in the cells' face and size. */
export type Measure = (text: string, bold: boolean) => number;

/** The width, in px, the Key column needs to hold any key, lit or not. */
export function keyColumnWidth(measure: Measure): number {
  const text = Math.max(...KEY_TEXTS.map((key) => measure(key, false)));
  const mark = Math.max(...MARKS.map((m) => measure(m, true)));
  return Math.ceil(text + BADGE_GAP + mark + BADGE_PADDING) + CELL_EDGES;
}

const cache = new Map<string, number>();

/**
 * `keyColumnWidth` in the face and size the list's number cells are drawn in
 * now (`--f-shell-mono`, `--f-size-shell-mono`, which the theme file sets),
 * read from the page. Undefined where nothing can be measured.
 */
export function measuredKeyColumnWidth(): number | undefined {
  if (typeof document === "undefined") return undefined;
  const probe = document.createElement("span");
  probe.style.cssText = "position:absolute;visibility:hidden;font-family:var(--f-shell-mono);font-size:var(--f-size-shell-mono)";
  document.body.append(probe);
  const { fontFamily, fontSize } = getComputedStyle(probe);
  probe.remove();
  const font = `${fontSize} ${fontFamily}`;
  const known = cache.get(font);
  if (known !== undefined) return known;
  const context = document.createElement("canvas").getContext("2d");
  if (!context) return undefined;
  const width = keyColumnWidth((text, bold) => {
    context.font = `${bold ? "bold " : ""}${font}`;
    return context.measureText(text).width;
  });
  cache.set(font, width);
  return width;
}
