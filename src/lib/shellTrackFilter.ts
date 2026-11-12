/**
 * The Track Filter bar's model (wbx-57): one filter, in the deck's Track
 * Filter fields (MyTags by category, Energy, Prep colours, BPM, keys, plus
 * Key match and BPM ±6% against the loaded track) and text, and what it asks
 * of each side.
 *
 * The same filter goes two ways and must narrow to the same tracks:
 *
 * - `toViewFilter`: the window's list, as `ViewSpec.query` and
 *   `ViewSpec.filter` (the Track Filter `rbl-index` applies; on a Stick list
 *   the same fields over the Stick's rows);
 * - `toServiceSelection` / `toCliArgs`: the Library service's `tracks.query`
 *   and `wreckordbox tracks query`, which resolve every field the way the
 *   window does (`crates/wreckordbox-service/src/track_filter.rs`).
 *
 * Meanings both sides share:
 *
 * - Energy: `energy` is a range of 1 to 5 or `null` for any Energy;
 *   `unrated` adds the tracks with none. An explicit 1 to 5 is every track
 *   that has an Energy.
 * - Keys are held by identity: a key on the Camelot wheel as its code
 *   (`8A` for `Am`, `8A` and Open Key `1m`), a name off the wheel as itself.
 *   An identity takes every spelling of the key in any list.
 * - BPM is whole BPMs as the list rounds them: 124 is 123.50 to 124.49. Both
 *   sides are sent the span in hundredths.
 * - Key match and BPM ±6% follow the loaded track and do nothing without
 *   one. With picks in the same field as well, both must hold.
 * - A MyTag category's All/Any is kept apart from its picks.
 *
 * Pure; nothing here touches a row or the backend.
 */
import type { Counted, KeyDisplay, TagCategory, TrackFilter } from "@/ipc/types";
import { toCamelot } from "@/lib/camelot";
import { keyLight } from "@/lib/keyLights";
import { formatKey } from "@/lib/preferences";

/** The MyTags picked in one category. */
export interface ShellTagPick {
  category: string;
  tags: string[];
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
  /** Categories switched to Any: a track carrying any picked tag passes. */
  anyCategories: string[];
  /** Energy 1 to 5, inclusive; `null` is any Energy. */
  energy: Range | null;
  /** Tracks with no Energy, as a choice of their own. */
  unrated: boolean;
  /** Colour names (Pink to Purple) and `none`; a track with any passes. */
  prepColours: string[];
  bpm: BpmRange | null;
  /** Key identities (see `keyIdentity`); a track in any of them passes. */
  keys: string[];
  /** Only keys that light against the loaded track's key. */
  keyMatch: boolean;
  /** Only tempos within 6% of the loaded track's. */
  bpmMatch: boolean;
}

/** The loaded track, as Key match and BPM ±6% follow it. */
export interface LoadedTrack {
  /** Its key as the key lights read it (after any key shift). */
  key: string | null;
  bpmX100: number | null;
}

export const NO_TRACK: LoadedTrack = { key: null, bpmX100: null };

export const EMPTY_SHELL_FILTER: ShellFilter = {
  text: "",
  myTags: [],
  anyCategories: [],
  energy: null,
  unrated: false,
  prepColours: [],
  bpm: null,
  keys: [],
  keyMatch: false,
  bpmMatch: false,
};

/** BPM ±6%, as the design draws it. */
export const BPM_MATCH_PCT = 6;
/** The lowest and highest whole BPM a range can reach. */
export const BPM_FLOOR = 1;
export const BPM_CEILING = 999;

export const ENERGY_LEVELS: readonly number[] = [1, 2, 3, 4, 5];

/** The choices of the Prep colour control: the eight colours, then none. */
export const PREP_CHOICES: readonly string[] = ["Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "none"];

/** A key's identity: its Camelot code when on the wheel, else itself. */
export function keyIdentity(stored: string): string {
  return toCamelot(stored) || stored;
}

/** Every code on the wheel, 1A, 1B, 2A ... 12B. */
const WHEEL: readonly string[] = Array.from({ length: 24 }, (_, i) => `${Math.floor(i / 2) + 1}${i % 2 === 0 ? "A" : "B"}`);

/** A key's place on the wheel: 1A, 1B, 2A ... 12B, then everything else. */
function wheelRank(identity: string): number {
  const code = toCamelot(identity);
  if (!code) return 1000;
  return Number(code.slice(0, -1)) * 2 + (code.endsWith("B") ? 1 : 0);
}

/** Key match's keys for the loaded key, in wheel order, or `null` when it is inert. */
export function matchedKeys(loaded: LoadedTrack): string[] | null {
  if (loaded.key === null || !toCamelot(loaded.key)) return null;
  return WHEEL.filter((code) => keyLight(loaded.key, code) !== "none");
}

/** BPM ±6%'s span around the loaded BPM in hundredths, or `null` when inert. */
export function bpmBand(loaded: LoadedTrack): [number, number] | null {
  if (loaded.bpmX100 === null || loaded.bpmX100 <= 0) return null;
  // As `rbl-index` widens a band: the spread rounded down, in hundredths.
  const spread = Math.floor((loaded.bpmX100 * BPM_MATCH_PCT) / 100);
  return [loaded.bpmX100 - spread, loaded.bpmX100 + spread];
}

/** The keys the filter admits, or `null` for no key constraint. */
function keyConstraint(f: ShellFilter, loaded: LoadedTrack): string[] | null {
  const matched = f.keyMatch ? matchedKeys(loaded) : null;
  if (f.keys.length === 0) return matched;
  if (matched === null) return [...f.keys];
  return f.keys.filter((key) => matched.includes(key)).sort((a, b) => wheelRank(a) - wheelRank(b));
}

/** The tempo span the filter admits in hundredths, or `null` for none. */
function bpmConstraint(f: ShellFilter, loaded: LoadedTrack): [number, number] | null {
  const band = f.bpmMatch ? bpmBand(loaded) : null;
  const picked: [number, number] | null = f.bpm === null
    ? null
    : [(f.bpm.min ?? BPM_FLOOR) * 100 - 50, (f.bpm.max ?? BPM_CEILING) * 100 + 49];
  if (band === null) return picked;
  if (picked === null) return band;
  const lo = Math.max(band[0], picked[0]);
  const hi = Math.min(band[1], picked[1]);
  // Apart: a span that admits nothing.
  return lo <= hi ? [lo, hi] : [1, 0];
}

function energyLevels(f: ShellFilter): number[] | null {
  if (f.energy === null && !f.unrated) return null;
  const levels = f.unrated ? [0] : [];
  if (f.energy !== null) for (let level = f.energy.min; level <= f.energy.max; level++) levels.push(level);
  return levels;
}

/** Whether the filter narrows the list at all, with this track loaded. */
export function isFiltering(f: ShellFilter, loaded: LoadedTrack): boolean {
  return (
    f.text.trim() !== "" ||
    f.myTags.length > 0 ||
    energyLevels(f) !== null ||
    f.prepColours.length > 0 ||
    bpmConstraint(f, loaded) !== null ||
    keyConstraint(f, loaded) !== null
  );
}

/** Whether anything at all is set, active or waiting for a loaded track. */
export function hasPicks(f: ShellFilter): boolean {
  return isFiltering(f, NO_TRACK) || f.keyMatch || f.bpmMatch || f.anyCategories.length > 0;
}

function toggled<T>(list: readonly T[], value: T): T[] {
  return list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
}

export function togglePrepColour(f: ShellFilter, colour: string): ShellFilter {
  return { ...f, prepColours: toggled(f.prepColours, colour) };
}

/** Picks or unpicks a tag in a category; a category left empty is dropped. */
export function toggleTag(f: ShellFilter, category: string, tag: string): ShellFilter {
  const at = f.myTags.findIndex((p) => p.category === category);
  if (at < 0) return { ...f, myTags: [...f.myTags, { category, tags: [tag] }] };
  const myTags = f.myTags
    .map((p, i) => (i === at ? { ...p, tags: toggled(p.tags, tag) } : p))
    .filter((p) => p.tags.length > 0);
  return { ...f, myTags };
}

/** A category's All/Any, kept whether or not anything is picked in it. */
export function setTagsAny(f: ShellFilter, category: string, any: boolean): ShellFilter {
  const others = f.anyCategories.filter((c) => c !== category);
  return { ...f, anyCategories: any ? [...others, category] : others };
}

export function tagsAny(f: ShellFilter, category: string): boolean {
  return f.anyCategories.includes(category);
}

/** Clears one category's picks; its All/Any stays. */
export function clearCategory(f: ShellFilter, category: string): ShellFilter {
  return { ...f, myTags: f.myTags.filter((p) => p.category !== category) };
}

/** Sets the Energy range, ends in either order; a `null` end is any Energy. */
export function setEnergyRange(f: ShellFilter, a: number | null, b: number | null): ShellFilter {
  if (a === null || b === null) return { ...f, energy: null };
  const min = Math.max(1, Math.min(a, b));
  const max = Math.min(5, Math.max(a, b));
  return { ...f, energy: { min, max } };
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
 * Drops the MyTag picks the Library no longer holds (`categories` is the
 * Library's, not one list's), naming each as the service does:
 * `"Category: Tag"`, or `"Category"` for a whole category.
 */
export function dropStale(f: ShellFilter, categories: readonly TagCategory[]): { filter: ShellFilter; dropped: string[] } {
  const dropped: string[] = [];
  const myTags: ShellTagPick[] = [];
  for (const pick of f.myTags) {
    const held = categories.find((c) => c.name === pick.category);
    if (!held) {
      dropped.push(pick.category);
      continue;
    }
    const tags = pick.tags.filter((tag) => {
      const kept = held.tags.includes(tag);
      if (!kept) dropped.push(`${pick.category}: ${tag}`);
      return kept;
    });
    if (tags.length > 0) myTags.push({ category: pick.category, tags });
  }
  const anyCategories = f.anyCategories.filter((c) => categories.some((held) => held.name === c));
  if (dropped.length === 0 && anyCategories.length === f.anyCategories.length) return { filter: f, dropped };
  return { filter: { ...f, myTags, anyCategories }, dropped };
}

/**
 * The window's list: the search text and the Track Filter.
 *
 * `categories` are the Library's MyTag categories in the order the window
 * lists them (`FilterValues.tags`); a category it no longer has points past
 * the end, which narrows to nothing.
 */
export function toViewFilter(
  f: ShellFilter,
  categories: readonly string[],
  loaded: LoadedTrack,
): { query: string; filter: TrackFilter | undefined } {
  const out: TrackFilter = {};
  const bpm = bpmConstraint(f, loaded);
  if (bpm !== null) out.bpm = { values: [], tolerancePct: 0, masterBpmX100: null, rangesX100: [bpm] };
  const keys = keyConstraint(f, loaded);
  if (keys !== null) out.keyIdentities = keys;
  const ratings = energyLevels(f);
  if (ratings !== null) out.ratings = ratings;
  if (f.prepColours.length > 0) out.colors = [...f.prepColours];
  if (f.myTags.length > 0) {
    out.tags = f.myTags.map((p) => {
      const at = categories.indexOf(p.category);
      return { category: at < 0 ? categories.length : at, any: tagsAny(f, p.category), tags: [...p.tags] };
    });
  }
  return { query: f.text.trim(), filter: Object.keys(out).length > 0 ? out : undefined };
}

/** Hundredths as `tracks.query` reads a tempo. */
function bpmText(span: [number, number]): string {
  return `${(span[0] / 100).toFixed(2)}..${(span[1] / 100).toFixed(2)}`;
}

function energyTexts(f: ShellFilter): string[] {
  const out: string[] = [];
  if (f.energy !== null) out.push(f.energy.min === f.energy.max ? `${f.energy.min}` : `${f.energy.min}..${f.energy.max}`);
  if (f.unrated) out.push("0");
  return out;
}

/** `tracks.query`'s arguments; `playlist` is the open playlist's identifier. */
export function toServiceSelection(f: ShellFilter, loaded: LoadedTrack, playlist?: string): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  const text = f.text.trim();
  if (text !== "") out.search = text;
  if (playlist !== undefined) out.playlist = playlist;
  const keys = keyConstraint(f, loaded);
  if (keys !== null) out.keys = keys;
  if (f.prepColours.length > 0) out.prep_colours = [...f.prepColours];
  const energies = energyTexts(f);
  if (energies.length > 0) out.energies = energies;
  const bpm = bpmConstraint(f, loaded);
  if (bpm !== null) out.bpm_ranges = [bpmText(bpm)];
  if (f.myTags.length > 0) {
    out.my_tag_picks = f.myTags.map((p) => ({ category: p.category, tags: [...p.tags], any: tagsAny(f, p.category) }));
  }
  return out;
}

/**
 * The same query as `wreckordbox tracks query` arguments. A key constraint
 * that came out empty (Key match and picked keys apart) has no CLI form.
 */
export function toCliArgs(f: ShellFilter, loaded: LoadedTrack, playlist?: string): string[] {
  const out: string[] = [];
  const text = f.text.trim();
  if (text !== "") out.push("--search", text);
  if (playlist !== undefined) out.push("--playlist", playlist);
  for (const key of keyConstraint(f, loaded) ?? []) out.push("--key", key);
  for (const colour of f.prepColours) out.push("--prep-colour", colour);
  for (const energy of energyTexts(f)) out.push("--energy", energy);
  const bpm = bpmConstraint(f, loaded);
  if (bpm !== null) out.push("--bpm-range", bpmText(bpm));
  for (const p of f.myTags) {
    for (const tag of p.tags) out.push("--mytag-in", p.category, tag);
    if (tagsAny(f, p.category)) out.push("--mytag-any", p.category);
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

export function pickSummaries(f: ShellFilter, _loaded: LoadedTrack, keyName: (identity: string) => string = (k) => k): PickSummaries {
  const range = (min: number, max: number) => (min === max ? `${min}` : `${min}-${max}`);
  let bpm: string | null = null;
  if (f.bpm !== null) {
    const { min, max } = f.bpm;
    bpm = min !== null && max !== null ? range(min, max) : min !== null ? `${min}+` : `-${max ?? ""}`;
  }
  const energy = [f.energy === null ? null : range(f.energy.min, f.energy.max), f.unrated ? "unrated" : null]
    .filter((part): part is string => part !== null);
  return {
    myTags: Object.fromEntries(f.myTags.map((p) => [p.category, p.tags.join(tagsAny(f, p.category) ? " or " : ", ")])),
    energy: energy.length > 0 ? energy.join(", ") : null,
    bpm,
    keys: f.keys.length > 0 ? f.keys.map(keyName).join(", ") : null,
    prepColours: f.prepColours.length > 0 ? f.prepColours.map((c) => (c === "none" ? "None" : c)).join(", ") : null,
  };
}

/** One key the bar offers: its identity, as the list writes it, and how many tracks of the list are in it. */
export interface KeyChoice {
  identity: string;
  label: string;
  count: number;
}

/**
 * The keys the bar offers, one per identity, written and ordered as the list
 * writes and sorts keys (wbx-56): `formatKey` with the view's key display;
 * wheel order when the key sort is `musical` (or `auto` with Camelot shown),
 * otherwise Classic alphabetical by the shown name. `offered` is what the
 * open list holds; identities already picked are offered too (count 0 when
 * the list holds none of them), so a pick can always be seen and undone.
 */
export function keyChoices(
  offered: readonly Counted<string>[],
  picked: readonly string[],
  display: KeyDisplay,
  keySort: "auto" | "alphabetical" | "musical",
): KeyChoice[] {
  const byIdentity = new Map<string, KeyChoice>();
  const add = (identity: string, count: number) => {
    const choice = byIdentity.get(identity);
    if (choice) choice.count += count;
    else byIdentity.set(identity, { identity, label: formatKey(identity, display), count });
  };
  for (const { value, count } of offered) add(keyIdentity(value), count);
  for (const identity of picked) add(identity, 0);
  const choices = [...byIdentity.values()];
  const wheel = keySort === "musical" || (keySort === "auto" && display === "alphanumeric");
  const byName = (a: KeyChoice, b: KeyChoice) => {
    const left = a.label.toLowerCase();
    const right = b.label.toLowerCase();
    return left === right ? 0 : left < right ? -1 : 1;
  };
  // Wheel order is `rbl-index`'s `camelot_rank`: number, then A before B,
  // keys off the wheel last by name. Classic is the list's comparator: the
  // shown name, case-folded.
  return choices.sort(wheel ? (a, b) => wheelRank(a.identity) - wheelRank(b.identity) || byName(a, b) : byName);
}

export function keyChoicePicked(f: ShellFilter, choice: KeyChoice): boolean {
  return f.keys.includes(choice.identity);
}

export function toggleKeyChoice(f: ShellFilter, choice: KeyChoice): ShellFilter {
  return { ...f, keys: toggled(f.keys, choice.identity) };
}
