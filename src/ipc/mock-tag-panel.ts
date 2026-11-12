/**
 * The mock backend's Intake tag panel (wbx-65), answered as the Library
 * service answers `intake.tag_panel` and `intake.set_tags`: without a Tag
 * schema, the mock's MyTag categories, each deck-facing; with one, its deck
 * categories first (matched to MyTag categories by name, each term to a
 * MyTag by text), then its other dimensions, whose tags are the DJ's Facts.
 * Every write is checked against the description as it is now and the
 * version the panel showed, and is one undoable step of the mock's history
 * (a MyTag edit, or a Fact change Undo puts back).
 *
 * URL flags for the browser tests:
 * - `?tagSchema=five`: five dimensions under names the window has never
 *   seen: two deck-facing ("setting" and "parts", shown as the mock's MyTag
 *   categories Situation and Components), a genre tree, "feel" and "era".
 * - `?tagSchema=deckTree`: the genre tree on the deck as a "Sub-genre"
 *   category (added to the mock's MyTags), with `tag_parents` on
 *   (`&tagParents=0` for off), Situation, "vocals" (one tag) and "feel".
 *
 * Hooks on `window`: `__tagWrites()` lists every write asked;
 * `__holdTagWrites()` holds them until `__releaseTagWrites()`;
 * `__failNextTagWrite(message)`; `__failTagPanelReads(on)`; `__myTagsOf(id)`
 * (MyTag names, in order); `__tagFactsOf(id)`; `__moveToDeck(dimension)`
 * (the DJ edits the schema: that dimension now reaches the deck);
 * `__outsideFact(id, dimension, value)` (another client's Fact, announced).
 */
import type { RowDto } from "./types";

export interface MockTagPanelSource {
  all: readonly RowDto[];
  /** The MyTag ids a track carries. */
  tagsOf: (trackId: string) => readonly string[];
  categories: { id: string; name: string; tags: { id: string; name: string }[] }[];
  wait: <T>(value: T) => Promise<T>;
  ready: () => boolean;
  /** Records one undoable step of the mock's history, announcing the change. */
  recordEdit: (label: string, apply: () => void, undo: () => void) => Promise<unknown>;
  /** Sets a track's MyTags (ids) without recording. */
  setTags: (trackId: string, tags: string[]) => void;
}

interface SchemaTerm { id: string; name: string; parent?: string; deck?: string }
interface SchemaDimension { id: string; tree: boolean; single?: boolean; terms: SchemaTerm[] }
interface Schema { deck: { dimension: string; label: string }[]; tagParents: boolean; dimensions: SchemaDimension[] }

const FIVE: Schema = {
  deck: [
    { dimension: "setting", label: "Situation" },
    { dimension: "parts", label: "Components" },
  ],
  tagParents: true,
  dimensions: [
    { id: "genre", tree: true, terms: [
      { id: "house", name: "House" },
      { id: "house-deep", name: "Deep", parent: "house" },
      { id: "house-acid", name: "Acid", parent: "house" },
      { id: "techno", name: "Techno" },
    ] },
    { id: "era", tree: false, terms: [{ id: "nineties", name: "90s" }, { id: "modern", name: "Modern" }] },
    { id: "feel", tree: false, terms: [{ id: "dark", name: "Dark" }, { id: "bright", name: "Bright" }, { id: "hypnotic", name: "Hypnotic" }] },
    { id: "parts", tree: false, terms: [{ id: "synth", name: "Synth" }, { id: "vocal", name: "Vocal" }] },
    { id: "setting", tree: false, terms: [{ id: "peak", name: "Peak" }, { id: "warm-up", name: "Warm-up" }] },
  ],
};

const DECK_TREE = (tagParents: boolean): Schema => ({
  deck: [
    { dimension: "genre", label: "Sub-genre" },
    { dimension: "setting", label: "Situation" },
  ],
  tagParents,
  dimensions: [
    { id: "genre", tree: true, terms: [
      { id: "house", name: "House", deck: "House" },
      { id: "house-deep", name: "Deep", parent: "house", deck: "> Deep" },
      { id: "house-acid", name: "Acid", parent: "house", deck: "> Acid" },
      { id: "techno", name: "Techno", deck: "Techno" },
    ] },
    { id: "feel", tree: false, terms: [{ id: "dark", name: "Dark" }, { id: "bright", name: "Bright" }] },
    { id: "setting", tree: false, terms: [{ id: "peak", name: "Peak" }, { id: "warm-up", name: "Warm-up" }] },
    { id: "vocals", tree: false, single: true, terms: [{ id: "male", name: "Male" }, { id: "female", name: "Female" }] },
  ],
});

function readUrl(): URLSearchParams {
  return typeof location === "undefined" ? new URLSearchParams() : new URLSearchParams(location.search);
}

const readable = (name: string) => name.charAt(0).toUpperCase() + name.slice(1).replaceAll("_", " ");

/** A short stable digest of a string, as the service's versions are opaque. */
function digest(text: string): string {
  let hash = 5381;
  for (let i = 0; i < text.length; i += 1) hash = ((hash * 33) ^ text.charCodeAt(i)) >>> 0;
  return hash.toString(16);
}

interface RawTag { id: string; name: string; parent: string | null; depth: number; my_tag: string | null }
interface RawDimension { id: string; label: string; deck: boolean; tree: boolean; tag_parents: boolean; single: boolean; edit: "my_tag" | "fact"; tags: RawTag[]; category: string | null }

export function createMockTagPanel(source: MockTagPanelSource) {
  const url = readUrl();
  const which = url.get("tagSchema");
  let schema: Schema | null = which === "five" ? { ...FIVE, deck: [...FIVE.deck] } : which === "deckTree" ? DECK_TREE(url.get("tagParents") !== "0") : null;
  if (which === "deckTree" && !source.categories.some((c) => c.name === "Sub-genre")) {
    source.categories.push({ id: "9", name: "Sub-genre", tags: [
      { id: "t-house", name: "House" }, { id: "t-deep", name: "> Deep" }, { id: "t-acid", name: "> Acid" }, { id: "t-techno", name: "Techno" },
    ] });
  }
  /** The DJ's Facts of the vocabulary dimensions, by track then dimension (a list, or one text). */
  const facts = new Map<string, Map<string, string[] | string>>();
  const writes: { track: string; dimension: string; terms: string[]; dropUnmapped: string[] }[] = [];
  let holding = false;
  let held: (() => void)[] = [];
  let failing = false;
  let failNext: string | null = null;
  const sidecarListeners = new Set<(change: { trackId: string; kind: string }) => void>();

  const depthOf = (dimension: SchemaDimension, term: SchemaTerm): number => {
    let depth = 0;
    let at = term.parent;
    while (at !== undefined) {
      depth += 1;
      at = dimension.terms.find((t) => t.id === at)?.parent;
    }
    return depth;
  };

  const describe = (): RawDimension[] => {
    if (!schema) {
      return source.categories
        .filter((category) => category.tags.length > 0)
        .map((category) => ({
          id: category.id, label: category.name, deck: true, tree: false, tag_parents: false, single: false, edit: "my_tag" as const,
          tags: category.tags.map((tag) => ({ id: tag.id, name: tag.name, parent: null, depth: 0, my_tag: tag.id })),
          category: category.id,
        }));
    }
    const current = schema;
    const deck = current.deck.flatMap((view): RawDimension[] => {
      const dimension = current.dimensions.find((d) => d.id === view.dimension);
      if (!dimension) return [];
      const category = source.categories.find((c) => c.name === view.label);
      return [{
        id: dimension.id, label: view.label, deck: true, tree: dimension.tree, tag_parents: dimension.tree && current.tagParents,
        single: dimension.single ?? false, edit: "my_tag",
        tags: dimension.terms.map((term) => ({
          id: term.id, name: term.name, parent: term.parent ?? null, depth: depthOf(dimension, term),
          my_tag: category?.tags.find((tag) => tag.name === (term.deck ?? term.name))?.id ?? null,
        })),
        category: category?.id ?? null,
      }];
    });
    const rest = current.dimensions
      .filter((dimension) => !current.deck.some((view) => view.dimension === dimension.id))
      .map((dimension): RawDimension => ({
        id: dimension.id, label: readable(dimension.id), deck: false, tree: dimension.tree, tag_parents: false,
        single: dimension.single ?? false, edit: "fact",
        tags: dimension.terms.map((term) => ({ id: term.id, name: term.name, parent: term.parent ?? null, depth: depthOf(dimension, term), my_tag: null })),
        category: null,
      }));
    return [...deck, ...rest];
  };

  const ancestors = (dimension: RawDimension, id: string): string[] => {
    const found: string[] = [];
    let at = dimension.tags.find((tag) => tag.id === id)?.parent ?? null;
    while (at !== null && !found.includes(at)) {
      found.push(at);
      at = dimension.tags.find((tag) => tag.id === at)?.parent ?? null;
    }
    return found;
  };
  const mostSpecific = (dimension: RawDimension, selection: readonly string[]) => {
    const implied = new Set(selection.flatMap((id) => ancestors(dimension, id)));
    return dimension.tags.filter((tag) => selection.includes(tag.id) && !implied.has(tag.id)).map((tag) => tag.id);
  };
  const factValues = (track: string, dimension: string): string[] => {
    const value = facts.get(track)?.get(dimension);
    return value === undefined ? [] : typeof value === "string" ? [value] : [...value];
  };

  const stateOf = (track: string, dimension: RawDimension) => {
    if (dimension.edit === "my_tag") {
      const carried = source.tagsOf(track);
      const category = source.categories.find((c) => c.id === dimension.category);
      const inCategory = category ? category.tags.filter((tag) => carried.includes(tag.id)) : [];
      const mapped = dimension.tags.filter((tag) => tag.my_tag !== null && inCategory.some((t) => t.id === tag.my_tag)).map((tag) => tag.id);
      const unmapped = inCategory.filter((t) => !dimension.tags.some((tag) => tag.my_tag === t.id)).map((t) => ({ id: t.id, name: t.name }));
      return { set: mostSpecific(dimension, mapped), unmapped, raw: inCategory.map((t) => t.id).sort().join(",") };
    }
    const values = factValues(track, dimension.id);
    return {
      set: dimension.tags.filter((tag) => values.includes(tag.id)).map((tag) => tag.id),
      unmapped: values.filter((value) => !dimension.tags.some((tag) => tag.id === value)).map((value) => ({ id: value, name: value })),
      raw: JSON.stringify(facts.get(track)?.get(dimension.id) ?? null),
    };
  };

  const panel = (track: string | null) => {
    const dimensions = describe();
    let answer: unknown = null;
    if (track !== null) {
      if (!source.all.some((row) => row.id === track)) throw new Error(`no track has the id ${track}`);
      const set: Record<string, string[]> = {};
      const unmapped: Record<string, { id: string; name: string }[]> = {};
      const versions: Record<string, string> = {};
      for (const dimension of dimensions) {
        const state = stateOf(track, dimension);
        set[dimension.id] = state.set;
        if (state.unmapped.length > 0) unmapped[dimension.id] = state.unmapped;
        versions[dimension.id] = digest(JSON.stringify(dimension) + state.raw);
      }
      answer = { id: track, set, unmapped, versions, facts_unavailable: null };
    }
    return {
      source: schema ? "tag_schema" : "my_tags", schema_problem: null,
      schema_version: digest(JSON.stringify(dimensions)), dimensions, track: answer,
    };
  };

  const intakeTagPanel = async (track: string | null): Promise<unknown> => {
    if (!source.ready()) throw new Error("The library has not finished loading yet.");
    if (failing) throw new Error("The Library service did not answer.");
    return source.wait(panel(track));
  };

  const refuse = (message: string, code: string) => Object.assign(new Error(message), { detail: code });
  const announce = (trackId: string, kind: string) => { for (const listener of sidecarListeners) listener({ trackId, kind }); };

  const intakeSetTags = async (track: string, dimensionId: string, terms: readonly string[], version: string, dropUnmapped: readonly string[]): Promise<unknown> => {
    writes.push({ track, dimension: dimensionId, terms: [...terms], dropUnmapped: [...dropUnmapped] });
    if (holding) await new Promise<void>((resolve) => { held.push(resolve); });
    if (failNext !== null) {
      const message = failNext;
      failNext = null;
      throw refuse(message, "write_refused");
    }
    const dimension = describe().find((d) => d.id === dimensionId);
    const now = panel(track) as { track: { versions: Record<string, string> } };
    if (!dimension || now.track.versions[dimensionId] !== version) {
      throw refuse("The tags changed since they were shown; they have been read again: click again.", "tag_panel_stale");
    }
    for (const term of terms) {
      if (!dimension.tags.some((tag) => tag.id === term)) throw refuse(`${dimension.label} has no tag ${term}`, "tag_not_in_dimension");
    }
    const selection = mostSpecific(dimension, terms);
    if (dimension.single && selection.length > 1) throw refuse(`${dimension.label} takes one tag`, "tag_not_in_dimension");
    const state = stateOf(track, dimension);
    if (state.set.join() === selection.join() && dropUnmapped.length === 0) {
      return { changed: false, panel: panel(track), facts_not_recorded: [] };
    }
    if (dimension.edit === "my_tag") {
      const wanted = new Set<string>();
      for (const term of selection) {
        for (const id of [term, ...(dimension.tag_parents ? ancestors(dimension, term) : [])]) {
          const myTag = dimension.tags.find((tag) => tag.id === id)?.my_tag ?? null;
          if (myTag === null) throw refuse(`${id} is not on the deck yet`, "tag_not_settable");
          wanted.add(myTag);
        }
      }
      const mapped = new Set(dimension.tags.flatMap((tag) => (tag.my_tag === null ? [] : [tag.my_tag])));
      const before = [...source.tagsOf(track)];
      const after = before.filter((id) => !dropUnmapped.includes(id) && (!mapped.has(id) || wanted.has(id)));
      for (const id of wanted) if (!after.includes(id)) after.push(id);
      await source.recordEdit("Track Edit", () => source.setTags(track, after), () => source.setTags(track, before));
    } else {
      const kept = state.unmapped.map((value) => value.id).filter((value) => !dropUnmapped.includes(value));
      const before = facts.get(track)?.get(dimension.id);
      const after: string[] | string | undefined = dimension.single
        ? (selection[0] ?? kept[0])
        : [...selection, ...kept];
      const put = (value: string[] | string | undefined) => {
        let of = facts.get(track);
        if (!of) { of = new Map(); facts.set(track, of); }
        if (value === undefined) of.delete(dimension.id);
        else of.set(dimension.id, value);
        announce(track, dimension.id);
      };
      await source.recordEdit("Track Edit", () => put(after), () => put(before));
    }
    return { changed: true, panel: panel(track), facts_not_recorded: [] };
  };

  if (typeof window !== "undefined") {
    const hooks = window as unknown as Record<string, unknown>;
    hooks.__tagWrites = () => writes.map((write) => ({ ...write, terms: [...write.terms] }));
    hooks.__holdTagWrites = () => { holding = true; };
    hooks.__releaseTagWrites = () => {
      holding = false;
      const waiting = held;
      held = [];
      for (const release of waiting) release();
    };
    hooks.__failNextTagWrite = (message: string) => { failNext = message; };
    hooks.__failTagPanelReads = (on: boolean) => { failing = on; };
    hooks.__myTagsOf = (id: string) => source.tagsOf(id).map((tag) => source.categories.flatMap((c) => c.tags).find((t) => t.id === tag)?.name ?? tag);
    hooks.__tagFactsOf = (id: string) => Object.fromEntries([...(facts.get(id) ?? new Map<string, string[] | string>())].map(([k, v]) => [k, typeof v === "string" ? v : [...v]]));
    hooks.__moveToDeck = (dimension: string) => {
      if (!schema) return;
      const label = readable(dimension);
      schema = { ...schema, deck: [...schema.deck, { dimension, label }] };
    };
    hooks.__outsideFact = (id: string, dimension: string, value: string[] | string) => {
      let of = facts.get(id);
      if (!of) { of = new Map(); facts.set(id, of); }
      of.set(dimension, value);
      announce(id, dimension);
    };
  }

  const onSidecarChanged = (listener: (change: { trackId: string; kind: string }) => void) => {
    sidecarListeners.add(listener);
    return () => { sidecarListeners.delete(listener); };
  };

  return { intakeTagPanel, intakeSetTags, onSidecarChanged };
}
