/**
 * The mock's `saveFilterPlaylist` (wbx-58): the request turned back into
 * the list filter the mock applies, so a saved intelligent playlist lists
 * what the filter showed on All Tracks, or refused in the backend's words
 * (`src-tauri/src/filter_playlist.rs`, the reference). The mock does not
 * expand a key into its spellings: one picked key is one choice here.
 */
import type { SaveFilterRequest, TrackFilter } from "./types";

const COLOURS = ["Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple"];

function energyLevels(texts: readonly string[]): number[] {
  const levels = new Set<number>();
  for (const text of texts) {
    const [low, high] = text.includes("..")
      ? text.split("..").map((part, i) => (part.trim() === "" ? (i === 0 ? 1 : 5) : Number(part)))
      : [Number(text), Number(text)];
    for (let level = Math.min(low!, high!); level <= Math.max(low!, high!); level++) levels.add(level);
  }
  return [...levels].sort((a, b) => a - b);
}

function runs(levels: readonly number[]): number {
  return levels.filter((level, i) => i === 0 || levels[i - 1] !== level - 1).length;
}

export function mockSaveRule(
  request: SaveFilterRequest,
  categories: readonly string[],
): { filter: TrackFilter } | { refusal: string } {
  const text = request.search?.trim() ?? "";
  if (text !== "") {
    return { refusal: `An intelligent playlist cannot search for text, so it cannot keep "${text}". Clear the text to save this filter.` };
  }
  if (request.within !== undefined) {
    return { refusal: `An intelligent playlist chooses from the whole Library, so it cannot keep this filter to "${request.within}". Open All Tracks to save it.` };
  }
  const follows = (request.follows ?? []).map((f) => (f === "key" ? "Key match" : "BPM ±6%"));
  if (follows.length > 0) {
    return {
      refusal: `${follows.join(" and ")} ${follows.length === 1 ? "follows" : "follow"} the loaded track, which an intelligent playlist cannot. Turn it off, or pick keys and a BPM range, to save this filter.`,
    };
  }

  const filter: TrackFilter = {};
  const fields: { what: string; choices: number }[] = [];
  const keys = request.keys ?? [];
  if (keys.length > 0) {
    filter.keyIdentities = [...keys];
    fields.push({ what: `${keys.length} keys (${keys.join(", ")})`, choices: keys.length });
  }
  const colours = request.prep_colours ?? [];
  if (colours.length > 0) {
    filter.colors = [...colours];
    const shown = colours.map((c) => (c === "none" ? "None" : COLOURS.find((k) => k.toLowerCase() === c.toLowerCase()) ?? c));
    fields.push({ what: `${colours.length} Prep colours (${shown.join(", ")})`, choices: colours.length });
  }
  const levels = energyLevels(request.energies ?? []);
  if (levels.length > 0) {
    filter.ratings = levels;
    fields.push({ what: "Energy", choices: runs(levels) });
  }
  for (const text of request.bpm_ranges ?? []) {
    const [low, high] = text.split("..").map((part) => Math.round(Number(part) * 100));
    filter.bpm = { values: [], tolerancePct: 0, masterBpmX100: null, rangesX100: [[low!, high!]] };
    fields.push({ what: `BPM ${text}`, choices: 1 });
  }
  for (const pick of request.my_tag_picks ?? []) {
    const at = categories.indexOf(pick.category);
    if (at < 0) return { refusal: `The MyTag category ${pick.category} is no longer in the Library, so this filter cannot be saved.` };
    (filter.tags ??= []).push({ category: at, any: pick.any, tags: [...pick.tags] });
    if (pick.any && pick.tags.length > 1) fields.push({ what: `${pick.category}: any of ${pick.tags.join(", ")}`, choices: pick.tags.length });
    else for (const tag of pick.tags) fields.push({ what: `${pick.category}: ${tag}`, choices: 1 });
  }

  if (fields.length === 0) return { refusal: "Nothing is picked in the Track Filter, so there is nothing to save." };
  const several = fields.filter((f) => f.choices > 1);
  if (several.length > 1 || (several.length === 1 && fields.length > 1)) {
    const named = several.map((f) => f.what);
    const list = named.length === 1 ? named[0] : `${named.slice(0, -1).join(", ")} and ${named[named.length - 1]}`;
    const rest = several.length === fields.length ? "together" : "together with the rest of this filter";
    return {
      refusal: `An intelligent playlist joins all its conditions with AND or all with OR, so it cannot hold ${list} ${rest}. Keep one choice in each field, or save that field alone.`,
    };
  }
  return { filter };
}
