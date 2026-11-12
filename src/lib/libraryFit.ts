/**
 * The Library view's columns at the width they get (wbx-60;
 * docs/design/tokens.md "Breakpoints", docs/design/screens/library
 * "Sizes"). What is drawn is decided here from the columns the DJ chose
 * (never changing that choice, which stays as stored):
 *
 * - by the shell's width: above 800 CSS px every chosen column; at 800 and
 *   below title, BPM, key, Energy and time; at 520 and below (the design's
 *   "about 436") title, BPM, key and Energy;
 * - with the second list shown, the main list drops date added and narrows
 *   MyTags;
 * - then, while what is drawn is still wider than the list (the title at
 *   its narrowest), columns give way in a fixed order; the four essentials
 *   never do.
 *
 * The title is the column that gives room: never narrower than
 * `TITLE_MIN_PX`, and at half and narrow widths it takes all that is left.
 */
import type { ColumnKey, ColumnSpec } from "./columns";
import { SHELL_NARROW_MAX_PX } from "./shellViews";

export type LibraryWidth = "full" | "half" | "narrow";

/** The narrow breakpoint (the compact player's too): the design's "about 436" window. */
export const LIBRARY_NARROW_MAX_PX = 520;
/** The title's narrowest, in px at 100%: a few words still read. */
export const TITLE_MIN_PX = 96;
/** MyTags beside the second list (the design: "narrows MyTags"). */
export const TWO_LIST_MYTAGS_PX = 120;

/** The essentials (requirements: "lists at narrow widths show title, BPM, key and Energy only"). */
const ESSENTIALS: readonly ColumnKey[] = ["title", "bpm", "key", "rating"];
const HALF: readonly ColumnKey[] = [...ESSENTIALS, "duration"];
/** What gives way first when the list is still too narrow: the design's three, then the extras, then `#` and time. */
export const GIVE_WAY: readonly ColumnKey[] = [
  "dateAdded", "myTag", "artist", "preview", "bitrate", "label", "album", "artwork", "trackNo", "duration",
];

export interface LibraryFitOptions {
  width: LibraryWidth;
  twoLists: boolean;
}

/** The column that gives room: at least `min` px; `grow` takes what is left of the row. */
export interface ColumnFill {
  key: ColumnKey;
  min: number;
  grow: boolean;
}

export interface LibraryFit {
  columns: readonly ColumnSpec[];
  fill: ColumnFill;
}

/** The width class of a shell `width` CSS px wide; `null` (not measured yet) is full. */
export function libraryWidthOf(width: number | null): LibraryWidth {
  if (width === null) return "full";
  if (width <= LIBRARY_NARROW_MAX_PX) return "narrow";
  if (width <= SHELL_NARROW_MAX_PX) return "half";
  return "full";
}

/** What a row of `columns` needs at least, the title at its narrowest. */
function needs(columns: readonly ColumnSpec[]): number {
  return columns.reduce((total, column) => total + (column.key === "title" ? Math.min(column.width, TITLE_MIN_PX) : column.width), 0);
}

/**
 * The columns to draw from `drawn` (the chosen ones at their drawn widths,
 * in the DJ's order) in a list `available` px wide.
 */
export function fitLibraryColumns(drawn: readonly ColumnSpec[], available: number, options: LibraryFitOptions): LibraryFit {
  let columns = [...drawn];
  if (options.width === "narrow") columns = columns.filter((column) => ESSENTIALS.includes(column.key));
  else if (options.width === "half") columns = columns.filter((column) => HALF.includes(column.key));
  else if (options.twoLists) {
    columns = columns
      .filter((column) => column.key !== "dateAdded")
      .map((column) => (column.key === "myTag" ? { ...column, width: Math.min(column.width, TWO_LIST_MYTAGS_PX) } : column));
  }
  for (const key of GIVE_WAY) {
    if (needs(columns) <= available) break;
    columns = columns.filter((column) => column.key !== key);
  }
  return { columns, fill: { key: "title", min: TITLE_MIN_PX, grow: options.width !== "full" } };
}

/**
 * Where a column moved to place `to` among the `drawn` columns goes among
 * the `chosen` ones (some of which may not be drawn): the place of the
 * drawn column it lands on.
 */
export function layoutIndexFor(drawn: readonly ColumnKey[], chosen: readonly ColumnKey[], to: number): number {
  if (drawn.length === 0) return to;
  const at = drawn[Math.max(0, Math.min(to, drawn.length - 1))]!;
  const index = chosen.indexOf(at);
  return index === -1 ? to : index;
}
