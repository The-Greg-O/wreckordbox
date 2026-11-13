import { describe, expect, it } from "vitest";

import type { ColumnSpec } from "./columns";
import { resolveLibrary, libraryDefaultLayout } from "./libraryColumns";
import { fitLibraryColumns, layoutIndexFor, libraryWidthOf, TITLE_MIN_PX, TWO_LIST_MYTAGS_PX } from "./libraryFit";

const defaults = (): ColumnSpec[] => resolveLibrary(libraryDefaultLayout());
const keys = (columns: readonly ColumnSpec[]) => columns.map((c) => c.key);
const sum = (columns: readonly ColumnSpec[]) => columns.reduce((a, c) => a + c.width, 0);

describe("libraryWidthOf", () => {
  it("follows the shell's breakpoints: above 800 full, 800 and below half, 520 and below narrow", () => {
    expect(libraryWidthOf(null)).toBe("full");
    expect(libraryWidthOf(801)).toBe("full");
    expect(libraryWidthOf(800)).toBe("half");
    expect(libraryWidthOf(521)).toBe("half");
    expect(libraryWidthOf(520)).toBe("narrow");
    expect(libraryWidthOf(436)).toBe("narrow");
  });
});

describe("fitLibraryColumns", () => {
  it("at full width with room keeps every chosen column, the title flexing only down", () => {
    const fit = fitLibraryColumns(defaults(), 1100, { width: "full", twoLists: false });
    expect(keys(fit.columns)).toEqual(keys(defaults()));
    expect(fit.fill).toEqual({ key: "title", min: TITLE_MIN_PX, grow: false });
  });

  it("at half width draws title, BPM, key, Energy and time, the title taking the rest", () => {
    const fit = fitLibraryColumns(defaults(), 620, { width: "half", twoLists: false });
    expect(keys(fit.columns)).toEqual(["title", "bpm", "key", "rating", "duration"]);
    expect(fit.fill?.grow).toBe(true);
  });

  it("at narrow width draws title, BPM, key and Energy only", () => {
    const fit = fitLibraryColumns(defaults(), 420, { width: "narrow", twoLists: false });
    expect(keys(fit.columns)).toEqual(["title", "bpm", "key", "rating"]);
  });

  it("keeps the DJ's order among what remains", () => {
    const moved = defaults();
    const order = ["trackNo", "key", "title", "rating", "bpm", "duration"];
    const columns = order.map((k) => moved.find((c) => c.key === k)!);
    expect(keys(fitLibraryColumns(columns, 420, { width: "narrow", twoLists: false }).columns)).toEqual(["key", "title", "rating", "bpm"]);
  });

  it("with two lists drops date added and narrows MyTags", () => {
    const fit = fitLibraryColumns(defaults(), 2000, { width: "full", twoLists: true });
    expect(keys(fit.columns)).not.toContain("dateAdded");
    expect(fit.columns.find((c) => c.key === "myTag")!.width).toBe(TWO_LIST_MYTAGS_PX);
  });

  it("gives way in a fixed order until what is drawn fits, the four essentials always staying", () => {
    const fit = fitLibraryColumns(defaults(), 520, { width: "full", twoLists: true });
    const drawnMin = sum(fit.columns) - fit.columns.find((c) => c.key === "title")!.width + TITLE_MIN_PX;
    expect(drawnMin).toBeLessThanOrEqual(520);
    expect(keys(fit.columns)).not.toContain("myTag");
    for (const key of ["title", "bpm", "key", "rating"] as const) expect(keys(fit.columns)).toContain(key);
    // Too narrow even for the essentials: they stay, and the list scrolls.
    expect(keys(fitLibraryColumns(defaults(), 100, { width: "full", twoLists: false }).columns)).toEqual(["title", "bpm", "key", "rating"]);
  });
});

describe("layoutIndexFor", () => {
  it("turns a place among the drawn columns into a place among the chosen ones", () => {
    const chosen = ["trackNo", "title", "artist", "bpm", "key", "rating"] as const;
    const drawn = ["title", "bpm", "key", "rating"] as const;
    // Dropping before "key" (drawn index 2) is chosen index 4.
    expect(layoutIndexFor(drawn, chosen, 2)).toBe(4);
    expect(layoutIndexFor(drawn, chosen, 0)).toBe(1);
    // Past the end: after the last chosen column.
    expect(layoutIndexFor(drawn, chosen, 9)).toBe(5);
    // Nothing dropped: the same index.
    expect(layoutIndexFor(chosen, chosen, 3)).toBe(3);
  });
});
