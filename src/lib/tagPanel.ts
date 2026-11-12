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
  /** A deck-facing tree whose children carry their parents' MyTags on the deck. */
  tagParents: boolean;
  /** Takes one tag at most: choosing another replaces it. */
  single: boolean;
  edit: TagEdit;
  tags: PanelTag[];
}

/** A value a track holds that no tag of the dimension stands for. */
export interface Unmapped {
  id: string;
  name: string;
}

export interface TrackTags {
  id: string;
  /** By dimension id: the tags set, as tag ids. Ancestors in the tree are implied, not listed. */
  set: Record<string, string[]>;
  /** By dimension id: values no tag stands for; shown, never dropped unless the DJ says. */
  unmapped: Record<string, Unmapped[]>;
  /** By dimension id: what a write must hand back, so a stale click is refused. */
  versions: Record<string, string>;
  /** Why the DJ's Facts could not be read; the Fact dimensions are then unknown. */
  factsUnavailable: string | null;
}

export interface TagPanel {
  source: "tag_schema" | "my_tags";
  schemaProblem: string | null;
  /** Changes whenever the description does. */
  schemaVersion: string;
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
      tagParents: d.tag_parents === true,
      single: d.single === true,
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
  const record = (value: unknown): Raw => (typeof value === "object" && value !== null ? value as Raw : {});
  const set: Record<string, string[]> = {};
  const unmapped: Record<string, Unmapped[]> = {};
  const versions: Record<string, string> = {};
  if (track) {
    for (const [dimension, tags] of Object.entries(record(track.set))) set[dimension] = list(tags).map(str);
    for (const [dimension, values] of Object.entries(record(track.unmapped))) {
      unmapped[dimension] = list(values).map((value) => ({ id: str(record(value).id), name: str(record(value).name) }));
    }
    for (const [dimension, version] of Object.entries(record(track.versions))) versions[dimension] = str(version);
  }
  return {
    source: panel.source === "tag_schema" ? "tag_schema" : "my_tags",
    schemaProblem: strOrNull(panel.schema_problem),
    schemaVersion: str(panel.schema_version),
    dimensions: deckFirst,
    track: track ? { id: str(track.id), set, unmapped, versions, factsUnavailable: strOrNull(track.facts_unavailable) } : null,
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
 * The tags a dimension holds after clicking `id`, in the dimension's order,
 * whichever way it is stored (a deck-facing tree derives its MyTags from
 * this selection in the service). A set tag comes off; an implied one comes off by taking off
 * every tag that implies it; an off one goes on, replacing its ancestors
 * (a Fact names the most specific tags only).
 */
export function toggledTerms(dimension: PanelDimension, set: readonly string[], id: string): string[] {
  const state = tagState(dimension, set, id);
  // One tag at most: the click is the answer, or clears it.
  if (dimension.single) return state === "set" ? [] : [id];
  let next: string[];
  if (state === "set") next = set.filter((carried) => carried !== id);
  // Implied by a child: narrows to the parent alone (its descendants off).
  else if (state === "implied") next = [...set.filter((carried) => !ancestors(dimension, carried).includes(id)), id];
  else {
    const above = dimension.tree ? ancestors(dimension, id) : [];
    next = [...set.filter((carried) => !above.includes(carried)), id];
  }
  const order = dimension.tags.map((tag) => tag.id);
  return [...new Set(next)].sort((a, b) => order.indexOf(a) - order.indexOf(b));
}

/** The answer of one click (`intake.set_tags`). */
export interface TagsSet {
  changed: boolean;
  panel: TagPanel;
  /** Why the DJ's Fact was not recorded, each said; empty when it was. */
  factsNotRecorded: string[];
}

export function fromServiceTagsSet(raw: unknown): TagsSet {
  const set = (raw ?? {}) as Raw;
  return {
    changed: set.changed === true,
    panel: fromServicePanel(set.panel),
    factsNotRecorded: list(set.facts_not_recorded).map(str),
  };
}
