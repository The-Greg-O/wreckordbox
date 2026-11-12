import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { BINDINGS, sameChord, type Platform } from "./shortcuts";
import { KEY_PRESETS, presetChord } from "./keyPresets";
import {
  PALETTE_ENTRIES, PALETTE_OPEN_CHORD, filterPalette, paletteAvailability, paletteChord, paletteEntry, paletteKey,
  type PaletteContext,
} from "./palette";

const linux: Platform = { mac: false, linux: true };
const mac: Platform = { mac: true };
const windows: Platform = { mac: false };

const entry = (id: string) => {
  const found = paletteEntry(id);
  if (found === null) throw new Error(`no entry ${id}`);
  return found;
};

const context = (over: Partial<PaletteContext> = {}): PaletteContext => ({
  shell: false, readOnly: false, enabled: {}, has: () => true, ...over,
});

/**
 * Every item `src-tauri/src/menu.rs` builds, read from the file itself so a
 * new menu item with no palette entry fails here: the ids it gives its own
 * items, and the predefined items it adds by kind.
 */
function nativeMenuItems(): string[] {
  const source = readFileSync(new URL("../../src-tauri/src/menu.rs", import.meta.url), "utf8");
  const ids = new Set<string>();
  for (const match of source.matchAll(/MenuItemBuilder::with_id\(\s*"([^"]+)"/g)) ids.add(match[1] ?? "");
  for (const match of source.matchAll(/Row::Item\(\s*"([^"]+)"/g)) ids.add(match[1] ?? "");
  for (const match of source.matchAll(/PredefinedMenuItem::(\w+)\(/g)) ids.add(`predefined:${match[1] ?? ""}`);
  return [...ids].sort();
}

describe("the command palette's registry", () => {
  it("holds every item the native menus held", () => {
    const items = nativeMenuItems();
    // A sanity check on the reading itself: a handful of known items.
    expect(items).toEqual(expect.arrayContaining(["settings", "import", "relocate", "layout-two", "predefined:quit"]));
    // Hide and Hide Others exist only on macOS, which keeps its menu bar.
    const macOnly = new Set(["predefined:hide", "predefined:hide_others"]);
    const covered = new Set(PALETTE_ENTRIES.flatMap((entry) => (entry.menu === undefined ? [] : [entry.menu])));
    expect(items.filter((id) => !macOnly.has(id) && !covered.has(id))).toEqual([]);
  });

  it("names each entry once", () => {
    const ids = PALETTE_ENTRIES.map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe("finding an entry", () => {
  const ids = (query: string) => filterPalette(PALETTE_ENTRIES, query).map((found) => found.id);

  it("lists everything before anything is typed", () => {
    expect(ids("")).toHaveLength(PALETTE_ENTRIES.length);
  });

  it("narrows as the DJ types, by name, by menu and by other words", () => {
    // Relocate's own name first, then the File menu's missing-file manager
    // (found by its other words) before Auto Relocate, in menu order.
    expect(ids("relocate")).toEqual(["relocate", "missing", "auto-relocate"]);
    expect(ids("two-deck")).toEqual(["layout-two"]);
    expect(ids("playlist delete")).toEqual(["delete-playlist"]);
    expect(ids("PRO DJ")).toEqual(["pro-dj-link"]);
    expect(ids("nothing like this")).toEqual([]);
  });

  it("puts names that start with what was typed first", () => {
    expect(ids("re")[0]).toBe("redo");
  });

  it("finds an entry by its translated name", () => {
    const german = (text: string) => (text === "Backups" ? "Sicherungen" : text);
    expect(filterPalette(PALETTE_ENTRIES, "sicher", german).map((found) => found.id)).toEqual(["backups"]);
  });
});

describe("an entry's key", () => {
  it("is the key map's, with the DJ's own applied", () => {
    expect(paletteChord(entry("settings"))).toEqual({ key: ",", metaKey: true });
    expect(paletteChord(entry("settings"), { "menu.settings": { key: "p", metaKey: true } })).toEqual({ key: "p", metaKey: true });
    expect(paletteChord(entry("locate-loaded"))).toEqual({ key: "l", metaKey: true });
    expect(paletteChord(entry("analyze-track"))).toEqual({ key: "a", metaKey: true, shiftKey: true });
  });

  it("is a view's key for a view", () => {
    expect(paletteChord(entry("view-intake"))).toEqual({ key: "2" });
    expect(paletteChord(entry("view-intake"), { viewIntake: { key: "" } })).toBeNull();
  });

  it("is none for an entry with no key, or a key taken away", () => {
    expect(paletteChord(entry("backups"))).toBeNull();
    expect(paletteChord(entry("renumber-track-order"))).toBeNull();
    expect(paletteChord(entry("settings"), { "menu.settings": { key: "" } })).toBeNull();
  });
});

describe("the palette's keys", () => {
  it("open the palette on every platform", () => {
    expect(paletteKey({ key: "P", ctrlKey: true, shiftKey: true }, linux)).toBe("open");
    expect(paletteKey({ key: "P", ctrlKey: true, shiftKey: true }, windows)).toBe("open");
    expect(paletteKey({ key: "p", metaKey: true, shiftKey: true }, mac)).toBe("open");
    expect(paletteKey({ key: "p", ctrlKey: true }, linux)).toBeNull();
  });

  it("answer what the native menu answered, on Linux only, where there is no menu", () => {
    expect(paletteKey({ key: "O", ctrlKey: true, shiftKey: true }, linux)).toBe("import-folder");
    expect(paletteKey({ key: "F", ctrlKey: true, shiftKey: true }, linux)).toBe("fullscreen");
    expect(paletteKey({ key: "q", ctrlKey: true }, linux)).toBe("quit");
    expect(paletteKey({ key: "w", ctrlKey: true }, linux)).toBe("close-window");
    // The menu's accelerators answer these on macOS and Windows.
    expect(paletteKey({ key: "O", ctrlKey: true, shiftKey: true }, windows)).toBeNull();
    expect(paletteKey({ key: "o", metaKey: true, shiftKey: true }, mac)).toBeNull();
    // Keys the window's own handlers answer are left to them.
    expect(paletteKey({ key: ",", ctrlKey: true }, linux)).toBeNull();
    expect(paletteKey({ key: "z", ctrlKey: true }, linux)).toBeNull();
  });

  it("never take a key any preset gives a row of the key map", () => {
    const own = [PALETTE_OPEN_CHORD, ...PALETTE_ENTRIES.flatMap((found) =>
      found.key?.from === "chord" && found.key.handled === "palette" ? [found.key.chord] : [])];
    for (const preset of KEY_PRESETS) {
      for (const row of BINDINGS) {
        if (row.action === undefined) continue;
        const { chord } = presetChord(row, preset);
        for (const taken of own) expect(sameChord(chord, taken), `${preset} ${row.id}`).toBe(false);
      }
    }
  });
});

describe("whether an entry is available", () => {
  it("follows the Track and Playlist menus' rules", () => {
    expect(paletteAvailability(entry("relocate"), context())).toEqual({ available: false, why: "Select tracks in a list first" });
    expect(paletteAvailability(entry("relocate"), context({ enabled: { relocate: true } }))).toEqual({ available: true });
    expect(paletteAvailability(entry("delete-playlist"), context())).toEqual({ available: false, why: "Select a playlist in the tree first" });
  });

  it("is not for a Library edit while the Library is locked", () => {
    expect(paletteAvailability(entry("import"), context({ readOnly: true })).available).toBe(false);
    expect(paletteAvailability(entry("export-xml"), context({ readOnly: true })).available).toBe(true);
  });

  it("is not for a part of the old layout while the view shell is drawn", () => {
    expect(paletteAvailability(entry("layout-two"), context()).available).toBe(true);
    expect(paletteAvailability(entry("layout-two"), context({ shell: true }))).toEqual({ available: false, why: "Not in the view shell yet" });
  });

  it("is not for a command nothing on screen answers", () => {
    expect(paletteAvailability(entry("view-review"), context({ has: () => false }))).toEqual({ available: false, why: "Only in the view shell" });
    expect(paletteAvailability(entry("view-review"), context()).available).toBe(true);
  });

  it("is always for Preferences and the panes the views do not show", () => {
    for (const id of ["settings", "pro-dj-link", "device-settings", "backups", "about"]) {
      expect(paletteAvailability(entry(id), context({ shell: true, readOnly: true, has: () => false })).available, id).toBe(true);
    }
  });
});
