/**
 * The mock backend's Intake tag panel (wbx-65), answered as the Library
 * service's `intake.tag_panel` answers: without a Tag schema, the mock's
 * MyTag categories, each deck-facing; with one, its deck categories first
 * (matched to the MyTag categories by name, each term to a MyTag by text),
 * then its other dimensions, whose tags are the DJ's Facts.
 *
 * URL flags for the browser tests:
 * - `?tagSchema=five`: a schema of five dimensions under names the window
 *   has never seen: two deck-facing ("setting" and "parts", shown on the
 *   deck as the mock's MyTag categories Situation and Components), a genre
 *   tree, "feel" and "era".
 *
 * Hooks on `window`: `__tagFactWrites()` lists every Fact write asked, in
 * order; `__holdTagFactWrites()` holds them until `__releaseTagFactWrites()`;
 * `__failTagPanelReads(on)` makes reads fail; `__myTagsOf(id)` is what a
 * track carries; `__tagFactsOf(id)` its Fact dimensions.
 */
import type { RowDto } from "./types";

export interface MockTagPanelSource {
  all: readonly RowDto[];
  /** The MyTag ids a track carries. */
  tagsOf: (trackId: string) => readonly string[];
  categories: readonly { id: string; name: string; tags: readonly { id: string; name: string }[] }[];
  wait: <T>(value: T) => Promise<T>;
  ready: () => boolean;
}

interface SchemaTerm { id: string; name: string; parent?: string }
interface SchemaDimension { id: string; tree: boolean; terms: SchemaTerm[] }

/** The five-dimension schema: what the deck shows, by category name and MyTag text. */
const FIVE: { deck: { dimension: string; label: string }[]; dimensions: SchemaDimension[] } = {
  deck: [
    { dimension: "setting", label: "Situation" },
    { dimension: "parts", label: "Components" },
  ],
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

function readUrl(): URLSearchParams {
  return typeof location === "undefined" ? new URLSearchParams() : new URLSearchParams(location.search);
}

const readable = (name: string) => name.charAt(0).toUpperCase() + name.slice(1).replaceAll("_", " ");

export function createMockTagPanel(source: MockTagPanelSource) {
  const schema = readUrl().get("tagSchema") === "five" ? FIVE : null;
  /** The DJ's Facts of the vocabulary dimensions, by track then dimension. */
  const facts = new Map<string, Map<string, string[]>>();
  const writes: { track: string; dimension: string; terms: string[] }[] = [];
  let holding = false;
  let held: (() => void)[] = [];
  let failing = false;

  const depthOf = (dimension: SchemaDimension, term: SchemaTerm): number => {
    let depth = 0;
    let at = term.parent;
    while (at !== undefined) {
      depth += 1;
      at = dimension.terms.find((t) => t.id === at)?.parent;
    }
    return depth;
  };

  const describe = () => {
    if (!schema) {
      return source.categories
        .filter((category) => category.tags.length > 0)
        .map((category) => ({
          id: category.id, label: category.name, deck: true, tree: false, edit: "my_tag",
          tags: category.tags.map((tag) => ({ id: tag.id, name: tag.name, parent: null, depth: 0, my_tag: tag.id })),
        }));
    }
    const deck = schema.deck.flatMap((view) => {
      const dimension = schema.dimensions.find((d) => d.id === view.dimension);
      if (!dimension) return [];
      const category = source.categories.find((c) => c.name === view.label);
      return [{
        id: dimension.id, label: view.label, deck: true, tree: dimension.tree, edit: "my_tag",
        tags: dimension.terms.map((term) => ({
          id: term.id, name: term.name, parent: term.parent ?? null, depth: depthOf(dimension, term),
          my_tag: category?.tags.find((tag) => tag.name === term.name)?.id ?? null,
        })),
      }];
    });
    const rest = schema.dimensions
      .filter((dimension) => !schema.deck.some((view) => view.dimension === dimension.id))
      .map((dimension) => ({
        id: dimension.id, label: readable(dimension.id), deck: false, tree: dimension.tree, edit: "fact",
        tags: dimension.terms.map((term) => ({ id: term.id, name: term.name, parent: term.parent ?? null, depth: depthOf(dimension, term), my_tag: null })),
      }));
    return [...deck, ...rest];
  };

  const intakeTagPanel = async (track: string | null): Promise<unknown> => {
    if (!source.ready()) throw new Error("The library has not finished loading yet.");
    if (failing) throw new Error("The Library service did not answer.");
    const dimensions = describe();
    let answer: unknown = null;
    if (track !== null) {
      if (!source.all.some((row) => row.id === track)) throw new Error(`no track has the id ${track}`);
      const carried = source.tagsOf(track);
      const set: Record<string, string[]> = {};
      for (const dimension of dimensions) {
        set[dimension.id] = dimension.edit === "my_tag"
          ? dimension.tags.filter((tag) => tag.my_tag !== null && carried.includes(tag.my_tag)).map((tag) => tag.id)
          : [...(facts.get(track)?.get(dimension.id) ?? [])];
      }
      answer = { id: track, set, facts_unavailable: null };
    }
    return source.wait({ source: schema ? "tag_schema" : "my_tags", schema_problem: null, dimensions, track: answer });
  };

  const intakeSetTagFact = async (track: string, dimension: string, terms: readonly string[]): Promise<void> => {
    writes.push({ track, dimension, terms: [...terms] });
    if (holding) await new Promise<void>((resolve) => { held.push(resolve); });
    const known = schema?.dimensions.find((d) => d.id === dimension);
    if (!known || schema?.deck.some((view) => view.dimension === dimension)) throw new Error(`${dimension} is not a dimension set as a Fact`);
    const unknown = terms.find((term) => !known.terms.some((t) => t.id === term));
    if (unknown !== undefined) throw new Error(`${unknown} is not one of ${dimension}'s terms`);
    let of = facts.get(track);
    if (!of) { of = new Map(); facts.set(track, of); }
    of.set(dimension, [...terms]);
    await source.wait(undefined);
  };

  if (typeof window !== "undefined") {
    const hooks = window as unknown as {
      __tagFactWrites: () => typeof writes;
      __holdTagFactWrites: () => void;
      __releaseTagFactWrites: () => void;
      __failTagPanelReads: (on: boolean) => void;
      __myTagsOf: (id: string) => string[];
      __tagFactsOf: (id: string) => Record<string, string[]>;
    };
    hooks.__tagFactWrites = () => writes.map((write) => ({ ...write, terms: [...write.terms] }));
    hooks.__holdTagFactWrites = () => { holding = true; };
    hooks.__releaseTagFactWrites = () => {
      holding = false;
      const waiting = held;
      held = [];
      for (const release of waiting) release();
    };
    hooks.__failTagPanelReads = (on) => { failing = on; };
    hooks.__myTagsOf = (id) => [...source.tagsOf(id)];
    hooks.__tagFactsOf = (id) => Object.fromEntries([...(facts.get(id) ?? new Map<string, string[]>())].map(([k, v]) => [k, [...v]]));
  }

  return { intakeTagPanel, intakeSetTagFact };
}
