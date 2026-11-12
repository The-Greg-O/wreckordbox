/**
 * The Library view's columns (ticket #55): what the DJ chooses tracks by.
 *
 * The view shell's Library has its own small catalogue beside upstream's
 * (`@/lib/columns`, which the old layout keeps until #77): its own labels,
 * widths sized for the design's density, and its own stored choice. The keys
 * are upstream's, so the list (`TrackTable`) fetches and draws them as it
 * always has; Energy is the star rating (CONTEXT.md) and MyTags the
 * track's MyTags with their ids and categories (the `myTags` field). Prep colour is not a column: it is a stripe at the row's left
 * edge (`prepColour`).
 *
 * Moving and resizing reuse `moveColumn` and `resizeColumn` from
 * `@/lib/columns`, which only need the key to be a known column.
 */
import type { RowDto, RowMyTag } from "@/ipc/types";
import { MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH, type ColumnKey, type ColumnSpec, type Layout } from "@/lib/columns";
import { COLOR_NAMES } from "@/lib/trackFilter";

/**
 * Every column the Library view offers, in the order a column turned on
 * takes its place. Widths are at 100% scale for 12 px type; the defaults sum
 * to under 1000 px so they fit beside the tree in a 1280 px window.
 */
export const LIBRARY_CATALOGUE: readonly ColumnSpec[] = [
  { key: "trackNo", label: "#", width: 36, align: "right", sortable: true, fixed: true },
  { key: "artwork", label: "Artwork", width: 32, sortable: false },
  { key: "title", label: "Title", width: 260, sortable: true },
  { key: "artist", label: "Artist", width: 160, sortable: true },
  { key: "album", label: "Album", width: 180, sortable: true },
  { key: "label", label: "Label", width: 140, sortable: true },
  { key: "bpm", label: "BPM", width: 52, align: "right", sortable: true },
  { key: "key", label: "Key", width: 48, sortable: true },
  { key: "rating", label: "Energy", width: 76, sortable: true },
  { key: "myTag", label: "MyTags", width: 200, sortable: false },
  { key: "duration", label: "Time", width: 56, align: "right", sortable: true },
  { key: "bitrate", label: "Bitrate", width: 80, align: "right", sortable: true },
  { key: "dateAdded", label: "Date added", width: 100, align: "right", sortable: true },
  { key: "preview", label: "Preview", width: 128, sortable: false },
];

/** On show in a fresh profile, after `#`: design/requirements.md, "Library". */
const DEFAULT_ORDER: readonly ColumnKey[] = [
  "title", "artist", "bpm", "key", "rating", "myTag", "duration", "dateAdded",
];

/** Offered in the headings' menu, off by default. */
export const LIBRARY_OPTIONAL: readonly ColumnSpec[] = LIBRARY_CATALOGUE.filter(
  (column) => !column.fixed && !DEFAULT_ORDER.includes(column.key),
);

/** The columns the headings' menu lists: every one but `#`. */
export const LIBRARY_MENU: readonly ColumnSpec[] = LIBRARY_CATALOGUE.filter((column) => !column.fixed);

/** Cannot be turned off: a list without titles is unreadable. */
export const LIBRARY_REQUIRED: readonly ColumnKey[] = ["trackNo", "title"];

const BY_KEY = new Map(LIBRARY_CATALOGUE.map((column) => [column.key, column]));
const OFFERED = new Set(LIBRARY_MENU.map((column) => column.key));

/** Where the Library view keeps its choice, apart from the old layout's tables. */
export const LIBRARY_COLUMNS_STORAGE_KEY = "rbl.columns.v2.libraryView";

export function libraryDefaultLayout(): Layout {
  return { order: [...DEFAULT_ORDER], widths: {} };
}

/** The columns on show, `#` first, at their current widths. */
export function resolveLibrary(layout: Layout): ColumnSpec[] {
  const head = LIBRARY_CATALOGUE.filter((column) => column.fixed);
  const chosen = layout.order.flatMap((key) => {
    const spec = OFFERED.has(key) ? BY_KEY.get(key) : undefined;
    return spec ? [spec] : [];
  });
  return [...head, ...chosen].map((spec) => ({ ...spec, width: layout.widths[spec.key] ?? spec.width }));
}

/** Where a column turned on goes: before the first on show that follows it in the catalogue. */
function slotOf(order: readonly ColumnKey[], key: ColumnKey): number {
  const rank = (k: ColumnKey) => LIBRARY_CATALOGUE.findIndex((column) => column.key === k);
  const at = order.findIndex((other) => rank(other) > rank(key));
  return at === -1 ? order.length : at;
}

/** Shows or hides a column the view offers; the title and `#` stay. */
export function toggleLibraryColumn(layout: Layout, key: ColumnKey): Layout {
  if (!OFFERED.has(key) || LIBRARY_REQUIRED.includes(key)) return layout;
  if (layout.order.includes(key)) return { ...layout, order: layout.order.filter((k) => k !== key) };
  const order = [...layout.order];
  order.splice(slotOf(order, key), 0, key);
  return { ...layout, order };
}

/** Repairs a stored choice: unknown or repeated columns go, the title comes back. */
export function sanitiseLibrary(value: unknown): Layout {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return libraryDefaultLayout();
  const raw = value as { order?: unknown; widths?: unknown };
  const seen = new Set<ColumnKey>();
  const order = (Array.isArray(raw.order) ? raw.order : []).filter((key): key is ColumnKey => {
    if (typeof key !== "string" || !OFFERED.has(key as ColumnKey) || seen.has(key as ColumnKey)) return false;
    seen.add(key as ColumnKey);
    return true;
  });
  if (order.length === 0) return libraryDefaultLayout();
  if (!order.includes("title")) order.splice(slotOf(order, "title"), 0, "title");
  const widths: Partial<Record<ColumnKey, number>> = {};
  const rawWidths = typeof raw.widths === "object" && raw.widths !== null ? raw.widths : {};
  for (const [key, width] of Object.entries(rawWidths)) {
    if (!BY_KEY.has(key as ColumnKey) || typeof width !== "number" || !Number.isFinite(width)) continue;
    widths[key as ColumnKey] = Math.round(Math.min(Math.max(width, MIN_COLUMN_WIDTH), MAX_COLUMN_WIDTH));
  }
  return { order, widths };
}

/** The row's Prep colour as rekordbox names its label, or null for none. */
export function prepColour(row: Pick<RowDto, "extra">): string | null {
  const value = row.extra?.color;
  if (typeof value !== "number" || value < 1) return null;
  return COLOR_NAMES[value - 1] ?? null;
}

/** Energy, 0 (unrated) to 5, from the star rating. */
export function energyLevel(rating: number): number {
  return Math.min(5, Math.max(0, Math.round(rating)));
}

/**
 * The row's MyTags as the Library holds them (the `myTags` field), each with
 * its id and category (`myTags`, lifted by the IPC client). Never rebuilt
 * from the joined display text: a tag's name may hold a comma.
 */
export function rowMyTags(row: Pick<RowDto, "myTags">): readonly RowMyTag[] {
  return row.myTags ?? [];
}
