import { describe, expect, it } from "vitest";

import {
  energyLevel,
  LIBRARY_OPTIONAL,
  libraryDefaultLayout,
  rowMyTags,
  prepColour,
  resolveLibrary,
  sanitiseLibrary,
  toggleLibraryColumn,
} from "./libraryColumns";

const labels = (layout = libraryDefaultLayout()) => resolveLibrary(layout).map((column) => column.label);

describe("the Library view's columns", () => {
  it("opens with the columns the DJ chooses tracks by, in the agreed order", () => {
    expect(labels()).toEqual(["#", "Title", "Artist", "BPM", "Key", "Energy", "MyTags", "Time", "Date added"]);
  });

  it("fits its default columns beside the tree in a 1280 px window", () => {
    const total = resolveLibrary(libraryDefaultLayout()).reduce((sum, column) => sum + column.width, 0);
    expect(total).toBeLessThanOrEqual(1000);
  });

  it("offers artwork, album, label, bitrate and the preview waveform, all off by default", () => {
    expect(LIBRARY_OPTIONAL.map((column) => column.label).sort()).toEqual(
      ["Album", "Artwork", "Bitrate", "Label", "Preview"],
    );
    const shown = new Set(libraryDefaultLayout().order);
    for (const column of LIBRARY_OPTIONAL) expect(shown.has(column.key)).toBe(false);
  });

  it("puts a column turned on at its place in the list, not at the end", () => {
    expect(labels(toggleLibraryColumn(libraryDefaultLayout(), "album"))).toEqual(
      ["#", "Title", "Artist", "Album", "BPM", "Key", "Energy", "MyTags", "Time", "Date added"],
    );
    expect(labels(toggleLibraryColumn(libraryDefaultLayout(), "artwork"))[1]).toBe("Artwork");
  });

  it("turns a column off again, but never the title", () => {
    const off = toggleLibraryColumn(libraryDefaultLayout(), "artist");
    expect(labels(off)).not.toContain("Artist");
    expect(toggleLibraryColumn(libraryDefaultLayout(), "title")).toEqual(libraryDefaultLayout());
  });

  it("refuses a column the Library view does not offer", () => {
    expect(toggleLibraryColumn(libraryDefaultLayout(), "composer")).toEqual(libraryDefaultLayout());
  });

  it("reads a stored choice back, dropping what it cannot show", () => {
    const stored = { order: ["title", "composer", "album", "album", 7], widths: { album: 9000, composer: 100, title: "x" } };
    const layout = sanitiseLibrary(stored);
    expect(layout.order).toEqual(["title", "album"]);
    expect(layout.widths).toEqual({ album: 1200 });
  });

  it("falls back to the defaults for nothing stored or something broken", () => {
    expect(sanitiseLibrary(null)).toEqual(libraryDefaultLayout());
    expect(sanitiseLibrary("[]")).toEqual(libraryDefaultLayout());
    expect(sanitiseLibrary({ order: [] })).toEqual(libraryDefaultLayout());
  });

  it("puts the title back into a stored choice that lost it", () => {
    expect(sanitiseLibrary({ order: ["artist", "bpm"] }).order).toEqual(["title", "artist", "bpm"]);
  });
});

describe("a row's Prep colour, Energy and MyTags", () => {
  it("names rekordbox's label colour, and none for no colour", () => {
    expect(prepColour({ extra: { color: 1 } })).toBe("Pink");
    expect(prepColour({ extra: { color: 5 } })).toBe("Green");
    expect(prepColour({ extra: { color: 8 } })).toBe("Purple");
    expect(prepColour({ extra: { color: 0 } })).toBeNull();
    expect(prepColour({ extra: { color: 9 } })).toBeNull();
    expect(prepColour({})).toBeNull();
  });

  it("reads Energy from the star rating, 0 for unrated", () => {
    expect(energyLevel(0)).toBe(0);
    expect(energyLevel(3)).toBe(3);
    expect(energyLevel(5)).toBe(5);
    expect(energyLevel(7)).toBe(5);
    expect(energyLevel(-1)).toBe(0);
  });

  it("reads the MyTags the Library sends, each as itself", () => {
    const tags = [
      { id: "1", name: "Dark, Driving", category: "Mood" },
      { id: "2", name: "Vocal", category: "Sound" },
    ];
    expect(rowMyTags({ myTags: tags })).toEqual(tags);
    expect(rowMyTags({})).toEqual([]);
  });
});
