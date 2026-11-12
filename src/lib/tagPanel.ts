/**
 * The Intake tag panel's model (wbx-65): the description the Library
 * service reports (`intake.tag_panel`) in the view's terms, and the pure
 * rules for what a tag shows and what clicking it sets.
 *
 * Nothing here knows how many dimensions there are or what they are
 * called: the panel is drawn from whatever the service describes, the Tag
 * schema's dimensions or, until there is one, the Library's MyTag
 * categories.
 */

/** How a dimension's tags are set: the Library's MyTag edits, or the DJ's Fact. */
export type TagEdit = "my_tag" | "fact";

export interface PanelTag {
  id: string;
  name: string;
  /** In the tree: the tag this is a child of. A child implies its parents. */
  parent: string | null;
  depth: number;
  /** The Library's MyTag that stands for it (deck-facing dimensions); `null` when there is none yet. */
  myTag: string | null;
}

export interface PanelDimension {
  id: string;
  label: string;
  /** Reaches the deck as a MyTag category. */
  deck: boolean;
  tree: boolean;
  edit: TagEdit;
  tags: PanelTag[];
}

export interface TrackTags {
  id: string;
  /** By dimension id: the tags set, as tag ids. Ancestors in the tree are implied, not listed. */
  set: Record<string, string[]>;
  /** Why the DJ's Facts could not be read; the Fact dimensions are then unknown. */
  factsUnavailable: string | null;
}

export interface TagPanel {
  source: "tag_schema" | "my_tags";
  schemaProblem: string | null;
  /** Deck-facing first, each group in the service's order. */
  dimensions: PanelDimension[];
  track: TrackTags | null;
}

type Raw = Record<string, unknown>;
const str = (value: unknown): string =>
  typeof value === "string" ? value : typeof value === "number" || typeof value === "boolean" ? String(value) : "";
const strOrNull = (value: unknown): string | null => (typeof value === "string" ? value : null);
const list = (value: unknown): unknown[] => (Array.isArray(value) ? value : []);

/** The service's snake_case answer in the view's terms; unknown fields are ignored. */
export function fromServicePanel(raw: unknown): TagPanel {
  const panel = (raw ?? {}) as Raw;
  const dimensions = list(panel.dimensions).map((item): PanelDimension => {
    const d = item as Raw;
    return {
      id: str(d.id),
      label: str(d.label),
      deck: d.deck === true,
      tree: d.tree === true,
      edit: d.edit === "fact" ? "fact" : "my_tag",
      tags: list(d.tags).map((tagItem): PanelTag => {
        const tag = tagItem as Raw;
        return {
          id: str(tag.id),
          name: str(tag.name),
          parent: strOrNull(tag.parent),
          depth: typeof tag.depth === "number" ? tag.depth : 0,
          myTag: strOrNull(tag.my_tag),
        };
      }),
    };
  });
  const deckFirst = [...dimensions.filter((d) => d.deck), ...dimensions.filter((d) => !d.deck)];
  const track = panel.track as Raw | null | undefined;
  const set: Record<string, string[]> = {};
  if (track && typeof track.set === "object" && track.set !== null) {
    for (const [dimension, tags] of Object.entries(track.set as Raw)) set[dimension] = list(tags).map(str);
  }
  return {
    source: panel.source === "tag_schema" ? "tag_schema" : "my_tags",
    schemaProblem: strOrNull(panel.schema_problem),
    dimensions: deckFirst,
    track: track ? { id: str(track.id), set, factsUnavailable: strOrNull(track.facts_unavailable) } : null,
  };
}

/** The ancestors of `id` in `dimension`, nearest first. */
function ancestors(dimension: PanelDimension, id: string): string[] {
  const parentOf = new Map(dimension.tags.map((tag) => [tag.id, tag.parent]));
  const found: string[] = [];
  let at = parentOf.get(id) ?? null;
  while (at !== null && !found.includes(at)) {
    found.push(at);
    at = parentOf.get(at) ?? null;
  }
  return found;
}

export type TagState = "set" | "implied" | "off";

/** A tag is set when the track carries it, implied when a child of it is set (the tree), else off. */
export function tagState(dimension: PanelDimension, set: readonly string[], id: string): TagState {
  if (set.includes(id)) return "set";
  if (dimension.tree && set.some((carried) => ancestors(dimension, carried).includes(id))) return "implied";
  return "off";
}

/**
 * The tags a Fact dimension holds after clicking `id`, in the dimension's
 * order. A set tag comes off; an implied one comes off by taking off
 * every tag that implies it; an off one goes on, replacing its ancestors
 * (a Fact names the most specific tags only).
 */
export function toggledTerms(dimension: PanelDimension, set: readonly string[], id: string): string[] {
  const state = tagState(dimension, set, id);
  let next: string[];
  if (state === "set") next = set.filter((carried) => carried !== id);
  else if (state === "implied") next = set.filter((carried) => !ancestors(dimension, carried).includes(id));
  else {
    const above = dimension.tree ? ancestors(dimension, id) : [];
    next = [...set.filter((carried) => !above.includes(carried)), id];
  }
  const order = dimension.tags.map((tag) => tag.id);
  return [...new Set(next)].sort((a, b) => order.indexOf(a) - order.indexOf(b));
}
