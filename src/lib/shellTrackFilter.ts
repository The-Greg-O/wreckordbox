/**
 * The Track Filter bar's model (wbx-57): one filter, in the deck's Track
 * Filter fields (MyTags by category, Energy range, Prep colours, BPM range,
 * keys) plus text, and what it asks of each side.
 *
 * The same filter goes two ways and must narrow to the same tracks:
 *
 * - `toViewFilter`: the window's list, as `ViewSpec.query` and
 *   `ViewSpec.filter` (the Track Filter `rbl-index` applies);
 * - `toServiceSelection` / `toCliArgs`: the Library service's `tracks.query`
 *   and `wreckordbox tracks query`, which resolve names the way the window
 *   does (`crates/wreckordbox-service/src/track_filter.rs`).
 *
 * A BPM range is whole BPMs, as the deck's Track Filter lists them: 124 is
 * every track from 123.50 to 128.49 rounded to 124. The window sends each
 * whole BPM in the range; the service is sent the same span in hundredths.
 *
 * Pure; nothing here touches a row or the backend.
 */
import type { TrackFilter } from "@/ipc/types";

/** The MyTags picked in one category, and its AND / OR switch. */
export interface ShellTagPick {
  category: string;
  tags: string[];
  /** `true`: a track carrying any of the tags passes; `false`: every one. */
  any: boolean;
}

export interface Range {
  min: number;
  max: number;
}

export interface BpmRange {
  /** Whole BPMs; `null` is an open end. */
  min: number | null;
  max: number | null;
}

export interface ShellFilter {
  /** Free text, as the search box matches it: every word somewhere. */
  text: string;
  /** Only categories with something picked. */
  myTags: ShellTagPick[];
  /** Energy 1 to 5, inclusive; `null` is any Energy. */
  energy: Range | null;
  /** Colour names (Pink to Purple); a track with any of them passes. */
  prepColours: string[];
  bpm: BpmRange | null;
  /** Key names as the Library stores them; a track in any of them passes. */
  keys: string[];
}

export const EMPTY_SHELL_FILTER: ShellFilter = {
  text: "",
  myTags: [],
  energy: null,
  prepColours: [],
  bpm: null,
  keys: [],
};

/** The lowest and highest whole BPM a range can reach. */
export const BPM_FLOOR = 1;
export const BPM_CEILING = 999;

export const ENERGY_LEVELS: readonly number[] = [1, 2, 3, 4, 5];

/** Whether the filter narrows the list at all. */
export function isFiltering(f: ShellFilter): boolean {
  return (
    f.text.trim() !== "" ||
    f.myTags.length > 0 ||
    f.energy !== null ||
    f.prepColours.length > 0 ||
    f.bpm !== null ||
    f.keys.length > 0
  );
}

function toggled<T>(list: readonly T[], value: T): T[] {
  return list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
}

export function toggleKey(f: ShellFilter, key: string): ShellFilter {
  return { ...f, keys: toggled(f.keys, key) };
}

export function togglePrepColour(f: ShellFilter, colour: string): ShellFilter {
  return { ...f, prepColours: toggled(f.prepColours, colour) };
}

/** Picks or unpicks a tag in a category; a category left empty is dropped. */
export function toggleTag(f: ShellFilter, category: string, tag: string): ShellFilter {
  const at = f.myTags.findIndex((p) => p.category === category);
  if (at < 0) return { ...f, myTags: [...f.myTags, { category, tags: [tag], any: false }] };
  const myTags = f.myTags
    .map((p, i) => (i === at ? { ...p, tags: toggled(p.tags, tag) } : p))
    .filter((p) => p.tags.length > 0);
  return { ...f, myTags };
}

export function setTagsAny(f: ShellFilter, category: string, any: boolean): ShellFilter {
  return { ...f, myTags: f.myTags.map((p) => (p.category === category ? { ...p, any } : p)) };
}

/** Clears one category's picks. */
export function clearCategory(f: ShellFilter, category: string): ShellFilter {
  return { ...f, myTags: f.myTags.filter((p) => p.category !== category) };
}

/** Sets the Energy range, ends in either order; 1 to 5 is no constraint. */
export function setEnergyRange(f: ShellFilter, a: number, b: number): ShellFilter {
  const min = Math.max(1, Math.min(a, b));
  const max = Math.min(5, Math.max(a, b));
  return { ...f, energy: min === 1 && max === 5 ? null : { min, max } };
}

/** Sets the BPM range in whole BPMs, ends in either order; both open is none. */
export function setBpmRange(f: ShellFilter, a: number | null, b: number | null): ShellFilter {
  const clamp = (v: number | null) =>
    v === null || !Number.isFinite(v) ? null : Math.min(BPM_CEILING, Math.max(BPM_FLOOR, Math.round(v)));
  let min = clamp(a);
  let max = clamp(b);
  if (min !== null && max !== null && min > max) [min, max] = [max, min];
  return { ...f, bpm: min === null && max === null ? null : { min, max } };
}

/**
 * The window's list: the search text and the Track Filter.
 *
 * `categories` are the Library's MyTag categories in the order the window
 * lists them (`FilterValues.tags`); a category it no longer has points past
 * the end, which narrows to nothing as a stale pick should.
 */
export function toViewFilter(
  f: ShellFilter,
  categories: readonly string[],
): { query: string; filter: TrackFilter | undefined } {
  const out: TrackFilter = {};
  if (f.bpm !== null) {
    const lo = f.bpm.min ?? BPM_FLOOR;
    const hi = f.bpm.max ?? BPM_CEILING;
    out.bpm = { values: Array.from({ length: hi - lo + 1 }, (_, i) => lo + i), tolerancePct: 0, masterBpmX100: null };
  }
  if (f.keys.length > 0) out.keys = [...f.keys];
  if (f.energy !== null) {
    const { min, max } = f.energy;
    out.ratings = Array.from({ length: max - min + 1 }, (_, i) => min + i);
  }
  if (f.prepColours.length > 0) out.colors = [...f.prepColours];
  if (f.myTags.length > 0) {
    out.tags = f.myTags.map((p) => {
      const at = categories.indexOf(p.category);
      return { category: at < 0 ? categories.length : at, any: p.any, tags: [...p.tags] };
    });
  }
  return { query: f.text.trim(), filter: Object.keys(out).length > 0 ? out : undefined };
}

/** A whole BPM's lower and upper edges in BPM, as `tracks.query` reads them. */
function bpmSpan(range: BpmRange): string {
  const lo = (range.min ?? BPM_FLOOR) - 0.5;
  const hi = (range.max ?? BPM_CEILING) + 0.49;
  return `${lo.toFixed(2)}..${hi.toFixed(2)}`;
}

/** `tracks.query`'s arguments; `playlist` is the open playlist's identifier. */
export function toServiceSelection(f: ShellFilter, playlist?: string): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  const text = f.text.trim();
  if (text !== "") out.search = text;
  if (playlist !== undefined) out.playlist = playlist;
  if (f.keys.length > 0) out.keys = [...f.keys];
  if (f.prepColours.length > 0) out.prep_colours = [...f.prepColours];
  if (f.energy !== null) out.energy = `${f.energy.min}..${f.energy.max}`;
  if (f.bpm !== null) out.bpm = bpmSpan(f.bpm);
  if (f.myTags.length > 0) out.my_tag_picks = f.myTags.map((p) => ({ category: p.category, tags: [...p.tags], any: p.any }));
  return out;
}

/** The same query as `wreckordbox tracks query` arguments. */
export function toCliArgs(f: ShellFilter, playlist?: string): string[] {
  const out: string[] = [];
  const text = f.text.trim();
  if (text !== "") out.push("--search", text);
  if (playlist !== undefined) out.push("--playlist", playlist);
  for (const key of f.keys) out.push("--key", key);
  for (const colour of f.prepColours) out.push("--prep-colour", colour);
  if (f.energy !== null) out.push("--energy", `${f.energy.min}..${f.energy.max}`);
  if (f.bpm !== null) out.push("--bpm", bpmSpan(f.bpm));
  for (const p of f.myTags) {
    for (const tag of p.tags) out.push("--mytag-in", `${p.category}:${tag}`);
    if (p.any) out.push("--mytag-any", p.category);
  }
  return out;
}

/** What each field's control says when something is picked in it. */
export interface PickSummaries {
  myTags: Record<string, string>;
  energy: string | null;
  bpm: string | null;
  keys: string | null;
  prepColours: string | null;
}

export function pickSummaries(f: ShellFilter, keyName: (stored: string) => string = (k) => k): PickSummaries {
  const range = (min: number, max: number) => (min === max ? `${min}` : `${min}-${max}`);
  let bpm: string | null = null;
  if (f.bpm !== null) {
    const { min, max } = f.bpm;
    bpm = min !== null && max !== null ? range(min, max) : min !== null ? `${min}+` : `-${max ?? ""}`;
  }
  return {
    myTags: Object.fromEntries(f.myTags.map((p) => [p.category, p.tags.join(p.any ? " or " : ", ")])),
    energy: f.energy === null ? null : range(f.energy.min, f.energy.max),
    bpm,
    keys: f.keys.length > 0 ? f.keys.map(keyName).join(", ") : null,
    prepColours: f.prepColours.length > 0 ? f.prepColours.join(", ") : null,
  };
}
