import { describe, expect, it } from "vitest";

import { fromServicePanel, tagState, toggledTerms, type PanelDimension } from "./tagPanel";

const genre: PanelDimension = {
  id: "genre",
  label: "Genre",
  deck: false,
  tree: true,
  edit: "fact",
  tags: [
    { id: "house", name: "House", parent: null, depth: 0, myTag: null },
    { id: "house-deep", name: "Deep", parent: "house", depth: 1, myTag: null },
    { id: "house-deep-dub", name: "Dub", parent: "house-deep", depth: 2, myTag: null },
    { id: "techno", name: "Techno", parent: null, depth: 0, myTag: null },
  ],
};

describe("fromServicePanel", () => {
  it("puts the service's words into the view's and the deck-facing dimensions first", () => {
    const panel = fromServicePanel({
      source: "tag_schema",
      schema_problem: null,
      dimensions: [
        { id: "genre", label: "Genre", deck: false, tree: true, edit: "fact", tags: [{ id: "house", name: "House", parent: null, depth: 0, my_tag: null }] },
        { id: "feel", label: "Mood", deck: true, tree: false, edit: "my_tag", tags: [{ id: "dark", name: "Dark", parent: null, depth: 0, my_tag: "70" }] },
      ],
      track: { id: "1", set: { feel: ["dark"] }, facts_unavailable: "busy" },
      extra: "ignored",
    });
    expect(panel.source).toBe("tag_schema");
    expect(panel.dimensions.map((d) => d.label)).toEqual(["Mood", "Genre"]);
    expect(panel.dimensions[0]?.tags[0]?.myTag).toBe("70");
    expect(panel.track).toEqual({ id: "1", set: { feel: ["dark"] }, factsUnavailable: "busy" });
  });

  it("reads a panel with no track and a schema problem", () => {
    const panel = fromServicePanel({ source: "my_tags", schema_problem: "line 3", dimensions: [], track: null });
    expect(panel.schemaProblem).toBe("line 3");
    expect(panel.track).toBeNull();
  });
});

describe("tagState", () => {
  it("is set for a tag the track carries, implied for its ancestors, off otherwise", () => {
    const set = ["house-deep-dub"];
    expect(tagState(genre, set, "house-deep-dub")).toBe("set");
    expect(tagState(genre, set, "house-deep")).toBe("implied");
    expect(tagState(genre, set, "house")).toBe("implied");
    expect(tagState(genre, set, "techno")).toBe("off");
  });
});

describe("toggledTerms", () => {
  it("adds a tag in the dimension's order and takes off one that is set", () => {
    expect(toggledTerms(genre, ["techno"], "house")).toEqual(["house", "techno"]);
    expect(toggledTerms(genre, ["house", "techno"], "house")).toEqual(["techno"]);
  });

  it("names the most specific tag only: a child replaces its ancestors", () => {
    expect(toggledTerms(genre, ["house"], "house-deep")).toEqual(["house-deep"]);
  });

  it("turns off an implied tag by taking off what implies it", () => {
    expect(toggledTerms(genre, ["house-deep-dub", "techno"], "house")).toEqual(["techno"]);
    expect(toggledTerms(genre, ["house-deep-dub"], "house-deep")).toEqual([]);
  });

  it("works on any dimension without a tree", () => {
    const flat: PanelDimension = { ...genre, tree: false, tags: genre.tags.map((t) => ({ ...t, parent: null, depth: 0 })) };
    expect(toggledTerms(flat, ["house"], "house-deep")).toEqual(["house", "house-deep"]);
  });
});
