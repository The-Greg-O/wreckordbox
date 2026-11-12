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
  shell: false,
  readOnly: false,
  enabled: {},
  has: () => true,
  side: "main",
  history: { undo: true, redo: true },
  loaded: { a: true, b: true },
  ...over,
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

  it("holds every command the key map registers, with its scope (the same registry as useCommands)", () => {
    for (const row of BINDINGS) {
      if (row.action === undefined || row.alias === true) continue;
      const found = PALETTE_ENTRIES.find((candidate) =>
        (candidate.run.kind === "command" && candidate.run.action === row.action &&
          (candidate.run.scope ?? "") === (row.deck ?? candidate.run.scope ?? "")) ||
        (row.deck === undefined && candidate.action === row.action));
      expect(found, row.id).toBeDefined();
    }
    // Representative window actions that are not menu items.
    expect(entry("key-playPause")).toMatchObject({ label: "Play/Pause", group: "Player A", run: { kind: "command", action: "playPause", scope: "a" } });
    expect(entry("key-b.cue")).toMatchObject({ group: "Player B", run: { kind: "command", action: "cue", scope: "b" } });
    expect(entry("key-mute").run).toEqual({ kind: "command", action: "mute" });
    expect(entry("key-volumeUp").run).toEqual({ kind: "command", action: "volumeUp" });
    expect(entry("key-jumpForward4Beats").run).toEqual({ kind: "command", action: "jumpForward4Beats", scope: "a" });
    expect(entry("key-moveDown").run).toEqual({ kind: "command", action: "moveDown", scope: "browser" });
    expect(paletteChord(entry("key-playPause"))).toEqual({ key: " " });
    expect(paletteChord(entry("key-mute"))).toEqual({ key: "F10", metaKey: true });
  });

  it("holds every command a part of the window registers", () => {
    // Every Action in the union is either a key map row, a view, or named by
    // an entry; a new Action with no way to the palette fails here.
    const source = readFileSync(new URL("./shortcuts.ts", import.meta.url), "utf8");
    const from = source.indexOf("export type Action =");
    // The union ends at the first quoted member followed by a semicolon (a
    // comment inside it mentions ";").
    const union = source.slice(from, source.indexOf("\";\n", from) + 1);
    const actions = [...union.matchAll(/\|\s*"(\w+)"/g)].map((match) => match[1] ?? "");
    expect(actions.length).toBeGreaterThan(100);
    const reachable = new Set(PALETTE_ENTRIES.flatMap((candidate) => [
      ...(candidate.run.kind === "command" ? [candidate.run.action] : []),
      ...(candidate.action === undefined ? [] : [candidate.action]),
    ]));
    // `clearSelection` has no handler anywhere: nothing registers it.
    expect(actions.filter((action) => action !== "clearSelection" && !reachable.has(action as never))).toEqual([]);
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
    // Relocate's own name first, Auto Relocate's name next, then the File
    // menu's missing-file manager, found by its other words.
    expect(ids("relocate")).toEqual(["relocate", "auto-relocate", "missing"]);
    expect(ids("two-deck")[0]).toBe("layout-two");
    expect(ids("playlist delete")).toEqual(["delete-playlist"]);
    expect(ids("PRO DJ")).toEqual(["pro-dj-link"]);
    expect(ids("nothing like this")).toEqual([]);
  });

  it("puts names that start with what was typed first", () => {
    expect(ids("re")[0]).toBe("redo");
  });

  it("matches letters in order, not only whole words (fuzzy)", () => {
    expect(ids("prefs")[0]).toBe("settings");
    expect(ids("rmcol")[0]).toBe("remove-from-collection");
    expect(ids("fulscr")[0]).toBe("fullscreen");
    // Letters out of order do not match.
    expect(ids("sferp")).not.toContain("settings");
  });

  it("ranks a whole-word match above a scattered one", () => {
    const found = ids("mute");
    expect(found[0]).toBe("key-mute");
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

  it("runs a part of the old layout from the view shell by switching to the old layout", () => {
    expect(paletteAvailability(entry("layout-two"), context())).toEqual({ available: true });
    for (const id of ["info", "sub", "tempo-slider", "layout-one", "layout-two", "layout-simple", "layout-browser"]) {
      expect(paletteAvailability(entry(id), context({ shell: true })), id)
        .toEqual({ available: true, note: "Switches to the old layout" });
    }
  });

  it("offers the way between the two layouts", () => {
    expect(paletteAvailability(entry("use-old-layout"), context({ shell: true }))).toEqual({ available: true });
    expect(paletteAvailability(entry("use-old-layout"), context()).available).toBe(false);
    expect(paletteAvailability(entry("use-view-shell"), context())).toEqual({ available: true });
    expect(paletteAvailability(entry("use-view-shell"), context({ shell: true })).available).toBe(false);
  });

  it("is not for a Track or Playlist edit, or Analyze, under Library Protection", () => {
    const all = Object.fromEntries(["remove-from-collection", "relocate", "create-playlist", "delete-playlist",
      "sort-items", "show-in-finder", "export-playlist-txt"].map((id) => [id, true]));
    for (const id of ["remove-from-collection", "relocate", "create-playlist", "delete-playlist", "sort-items", "analyze-track"]) {
      expect(paletteAvailability(entry(id), context({ readOnly: true, enabled: all })), id)
        .toEqual({ available: false, why: "The Library is locked for editing" });
    }
    // Reading is never locked.
    expect(paletteAvailability(entry("show-in-finder"), context({ readOnly: true, enabled: all })).available).toBe(true);
    expect(paletteAvailability(entry("export-playlist-txt"), context({ readOnly: true, enabled: all })).available).toBe(true);
  });

  it("is not for Undo or Redo with nothing to undo or redo", () => {
    const empty = context({ history: { undo: false, redo: false } });
    expect(paletteAvailability(entry("undo"), empty)).toEqual({ available: false, why: "Nothing to undo" });
    expect(paletteAvailability(entry("redo"), empty)).toEqual({ available: false, why: "Nothing to redo" });
    expect(paletteAvailability(entry("undo"), context({ history: { undo: true, redo: false } })).available).toBe(true);
  });

  it("is not for a deck's or the loaded track's command with an empty player", () => {
    const empty = context({ loaded: { a: false, b: false } });
    expect(paletteAvailability(entry("locate-loaded"), empty)).toEqual({ available: false, why: "No track is loaded on a deck" });
    expect(paletteAvailability(entry("key-playPause"), empty)).toEqual({ available: false, why: "Load a track on Player A first" });
    expect(paletteAvailability(entry("key-cue"), empty).available).toBe(false);
    expect(paletteAvailability(entry("key-playPause"), context()).available).toBe(true);
    // The master's commands need no track.
    expect(paletteAvailability(entry("key-mute"), empty).available).toBe(true);
  });

  it("asks a deck's or a list's command of its own scope", () => {
    const asked: string[] = [];
    const has = (action: string, scope?: string) => {
      asked.push(`${action}@${scope ?? ""}`);
      return scope !== "b";
    };
    expect(paletteAvailability(entry("key-b.playPause"), context({ has })).available).toBe(false);
    expect(paletteAvailability(entry("key-playPause"), context({ has })).available).toBe(true);
    expect(paletteAvailability(entry("key-moveDown"), context({ has, side: "sub" })).available).toBe(true);
    expect(asked).toEqual(["playPause@b", "playPause@a", "moveDown@sub"]);
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
