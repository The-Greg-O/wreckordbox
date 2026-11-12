/**
 * The command palette's registry (ticket #54, ADR 0007).
 *
 * Every action the window offers is registered here once, with its name,
 * its key and when it is available. The palette lists these entries, the
 * palette's key handler reads their keys, and the palette's opener reads
 * its own; nothing else keeps a second copy of a name or a key.
 *
 * On Linux there is no native menu row (`src-tauri/src/menu.rs`,
 * `has_menu_row`), so every item the native menus held is an entry here,
 * and `palette.test.ts` reads `menu.rs` to keep it that way. An entry stands
 * for a native item through `menu`, and runs it the way a click on it did:
 * the window's own menu handler (`runMenu` in `App.tsx`) for the items the
 * window answered, the shell (`run_menu_item`) for the three it did itself.
 *
 * Where an entry's key comes from:
 *  - `binding`: a row of the key map (`BINDINGS` in `shortcuts.ts`), with
 *    the DJ's own keys applied; that row's handler answers the key.
 *  - `view`: one of the view shell's four views (`shellViews.ts`).
 *  - `chord`: the registry's own. `handled: "palette"` means the palette's
 *    key handler answers it where there is no native menu to (Linux);
 *    `handled: "window"` means the webview or the window's key handler
 *    already does, and the registry only names it.
 */
import { menuBarItem } from "./menuBar";
import { SHELL_VIEWS, shellViewChord } from "./shellViews";
import {
  BINDINGS, chordMatches, type Action, type KeyChord, type KeyOverrides, type Platform,
} from "./shortcuts";

/** The items the shell does itself (`SHELL_ITEMS` in `menu.rs`). */
export type ShellItem = "fullscreen" | "close-window" | "quit";

/** What running an entry does. */
export type PaletteRun =
  /** The window's own menu handler, with a native item's id. */
  | { kind: "menu"; id: string }
  /** The shell, for an item the window never heard of. */
  | { kind: "shell"; id: ShellItem }
  /** A registered command (`commands.ts`). */
  | { kind: "command"; action: Action }
  /** The Preferences window at a pane (`PreferencesTarget`). */
  | { kind: "preferences"; pane: string }
  /** A text field's own clipboard and selection, as the Edit menu's were. */
  | { kind: "edit"; edit: "cut" | "copy" | "paste" | "selectAll" };

export type PaletteKey =
  | { from: "binding"; id: string; handled: "window" | "palette" }
  | { from: "view"; action: Action }
  | { from: "chord"; chord: KeyChord; handled: "window" | "palette" };

export interface PaletteEntry {
  /** Stable, and the `data-entry` a test can find it by. */
  id: string;
  /** Its name, in English; drawn through the translation. */
  label: string;
  /** Where it sits: the native menu it came from, or Views or Preferences. */
  group: string;
  /** Other words it is found by. */
  keywords?: readonly string[];
  run: PaletteRun;
  key?: PaletteKey;
  /** The native menu item it stands for: the item's id, or `predefined:<kind>`. */
  menu?: string;
  /** Changes the Library: unavailable while it is locked. */
  writes?: boolean;
  /** Acts on a part of upstream's layout the view shell does not draw. */
  oldLayoutOnly?: boolean;
}

const menu = (id: string): PaletteRun => ({ kind: "menu", id });
const binding = (id: string, handled: "window" | "palette" = "window"): PaletteKey => ({ from: "binding", id, handled });
const chord = (value: KeyChord, handled: "window" | "palette"): PaletteKey => ({ from: "chord", chord: value, handled });

/** The key that opens the palette, on every platform. */
export const PALETTE_OPEN_CHORD: KeyChord = { key: "p", metaKey: true, shiftKey: true };

/** The Track and Playlist menus' items, in their menus' order (`menu.rs`). */
const TRACK_ITEMS: readonly [string, string][] = [
  ["import-to-collection", "Import To Collection"],
  ["analyze-track", "Analyze Track"],
  ["analysis-lock-on", "Analysis Lock On"],
  ["analysis-lock-off", "Analysis Lock Off"],
  ["reload-tag", "Reload Tag"],
  ["remove-from-collection", "Remove from Collection"],
  ["show-in-finder", "Show in Finder"],
  ["auto-relocate", "Auto Relocate"],
  ["relocate", "Relocate"],
];
const PLAYLIST_ITEMS: readonly [string, string][] = [
  ["create-playlist", "Create New Playlist"],
  ["create-intelligent-playlist", "Create New Intelligent Playlist"],
  ["create-folder", "Create New Folder"],
  ["delete-playlist", "Delete Playlist"],
  ["sort-items", "Sort Items"],
  ["renumber-track-order", "Renumber Track Order"],
  ["export-playlist-txt", "Export a playlist to a file (*.txt)"],
  ["export-playlist-m3u8", "Export a playlist to a file for music apps (*.m3u8)"],
];

/** The entry a Track or Playlist item gets: its key, if the key map gives its command one. */
function barEntry(group: string, [id, label]: readonly [string, string]): PaletteEntry {
  const action = id === "analyze-track" ? "analyseSelection" : menuBarItem(id)?.action;
  const row = BINDINGS.find((candidate) => candidate.action !== undefined && candidate.action === action);
  return { id, label, group, run: menu(id), menu: id, ...(row ? { key: binding(row.id) } : {}) };
}

/**
 * Every entry, in the order the palette lists them before anything is
 * typed: the views, then the native menus in their order, then the parts of
 * Preferences the four views do not show.
 */
export const PALETTE_ENTRIES: readonly PaletteEntry[] = [
  ...SHELL_VIEWS.map((view): PaletteEntry => ({
    id: `view-${view.id}`,
    label: `Show ${view.label}`,
    group: "Views",
    keywords: ["view", "tab"],
    run: { kind: "command", action: view.action },
    key: { from: "view", action: view.action },
  })),

  // wreckordbox (the application menu).
  {
    id: "settings", label: "Preferences", group: "wreckordbox", keywords: ["settings", "options"],
    run: menu("settings"), menu: "settings", key: binding("menu.settings"),
  },
  {
    id: "about", label: "About wreckordbox", group: "wreckordbox", keywords: ["version", "licence"],
    run: { kind: "preferences", pane: "about" }, menu: "predefined:about",
  },
  {
    id: "quit", label: "Quit wreckordbox", group: "wreckordbox", keywords: ["exit", "close"],
    run: { kind: "shell", id: "quit" }, menu: "predefined:quit",
    key: chord({ key: "q", metaKey: true }, "palette"),
  },

  // File.
  {
    id: "import", label: "Import", group: "File", keywords: ["add", "files"],
    run: menu("import"), menu: "import", key: binding("menu.import"), writes: true,
  },
  {
    id: "import-folder", label: "Import Folder…", group: "File", keywords: ["add"],
    run: menu("import-folder"), menu: "import-folder", writes: true,
    key: chord({ key: "o", metaKey: true, shiftKey: true }, "palette"),
  },
  {
    id: "import-xml", label: "Import rekordbox xml…", group: "File",
    run: menu("import-xml"), menu: "import-xml", writes: true,
  },
  {
    id: "import-itunes", label: "Import iTunes Library xml…", group: "File", keywords: ["music"],
    run: menu("import-itunes"), menu: "import-itunes", writes: true,
  },
  {
    id: "missing", label: "Display All Missing Files", group: "File", keywords: ["missing", "relocate"],
    run: menu("missing"), menu: "missing",
  },
  {
    id: "export-xml", label: "Export Collection in xml format…", group: "File",
    run: menu("export-xml"), menu: "export-xml",
  },
  {
    id: "close-window", label: "Close Window", group: "File",
    run: { kind: "shell", id: "close-window" }, menu: "predefined:close_window",
    key: chord({ key: "w", metaKey: true }, "palette"),
  },

  // Edit.
  {
    id: "undo", label: "Undo", group: "Edit", run: menu("undo"), menu: "undo",
    key: chord({ key: "z", metaKey: true }, "window"),
  },
  {
    id: "redo", label: "Redo", group: "Edit", run: menu("redo"), menu: "redo",
    key: chord({ key: "z", metaKey: true, shiftKey: true }, "window"),
  },
  {
    id: "cut", label: "Cut", group: "Edit", run: { kind: "edit", edit: "cut" }, menu: "predefined:cut",
    key: chord({ key: "x", metaKey: true }, "window"),
  },
  {
    id: "copy", label: "Copy", group: "Edit", run: { kind: "edit", edit: "copy" }, menu: "predefined:copy",
    key: chord({ key: "c", metaKey: true }, "window"),
  },
  {
    id: "paste", label: "Paste", group: "Edit", run: { kind: "edit", edit: "paste" }, menu: "predefined:paste",
    key: chord({ key: "v", metaKey: true }, "window"),
  },
  {
    id: "select-all", label: "Select All", group: "Edit", run: { kind: "edit", edit: "selectAll" },
    menu: "predefined:select_all", key: binding("selectAll"),
  },

  // Track and Playlist.
  ...TRACK_ITEMS.map((item) => barEntry("Track", item)),
  {
    id: "locate-loaded", label: "Locate track loaded on deck", group: "Track",
    run: menu("locate-loaded"), menu: "locate-loaded", key: binding("locateLoaded"),
  },
  ...PLAYLIST_ITEMS.map((item) => barEntry("Playlist", item)),

  // View.
  {
    id: "info", label: "Information Window", group: "View", run: menu("info"), menu: "info",
    key: binding("menu.info"), oldLayoutOnly: true,
  },
  {
    id: "sub", label: "Sub-Browser Window", group: "View", run: menu("sub"), menu: "sub",
    key: binding("menu.sub"), oldLayoutOnly: true,
  },
  {
    id: "tempo-slider", label: "Display Tempo slider", group: "View", run: menu("tempo-slider"),
    menu: "tempo-slider", oldLayoutOnly: true,
  },
  {
    id: "fullscreen", label: "Full screen", group: "View", run: { kind: "shell", id: "fullscreen" },
    menu: "fullscreen", key: binding("menu.fullscreen", "palette"),
  },
  {
    id: "layout-one", label: "1 Player", group: "View", keywords: ["layout", "deck"],
    run: menu("layout-one"), menu: "layout-one", key: binding("menu.layout-one"), oldLayoutOnly: true,
  },
  {
    id: "layout-two", label: "2 Players", group: "View", keywords: ["layout", "two-deck", "decks"],
    run: menu("layout-two"), menu: "layout-two", key: binding("menu.layout-two"), oldLayoutOnly: true,
  },
  {
    id: "layout-simple", label: "Simple Player", group: "View", keywords: ["layout", "deck"],
    run: menu("layout-simple"), menu: "layout-simple", key: binding("menu.layout-simple"), oldLayoutOnly: true,
  },
  {
    id: "layout-browser", label: "Full Browser", group: "View", keywords: ["layout"],
    run: menu("layout-browser"), menu: "layout-browser", key: binding("menu.layout-browser"), oldLayoutOnly: true,
  },
  {
    id: "ui-scale-smaller", label: "Smaller Interface", group: "View", keywords: ["scale", "zoom"],
    run: { kind: "command", action: "uiScaleSmaller" }, key: binding("uiScaleSmaller"),
  },
  {
    id: "ui-scale-larger", label: "Larger Interface", group: "View", keywords: ["scale", "zoom"],
    run: { kind: "command", action: "uiScaleLarger" }, key: binding("uiScaleLarger"),
  },
  {
    id: "ui-scale-reset", label: "Reset Interface Size", group: "View", keywords: ["scale", "zoom"],
    run: { kind: "command", action: "uiScaleReset" }, key: binding("uiScaleReset"),
  },
  {
    id: "search", label: "Search", group: "View", keywords: ["find"],
    run: { kind: "command", action: "focusSearch" }, key: binding("focusSearch"),
  },

  // What the four views do not show, kept as it was in Preferences.
  {
    id: "pro-dj-link", label: "PRO DJ LINK", group: "Preferences", keywords: ["link", "network", "cdj"],
    run: { kind: "preferences", pane: "link" },
  },
  {
    id: "device-settings", label: "Device Settings", group: "Preferences", keywords: ["dj system", "my settings", "stick"],
    run: { kind: "preferences", pane: "djSystem" },
  },
  {
    id: "backups", label: "Backups", group: "Preferences", keywords: ["restore", "archive"],
    run: { kind: "preferences", pane: "backups" },
  },
  {
    id: "keyboard", label: "Keyboard", group: "Preferences", keywords: ["keys", "shortcuts"],
    run: { kind: "preferences", pane: "keyboard" },
  },
];

/** The entry with an id, or null. */
export function paletteEntry(id: string): PaletteEntry | null {
  return PALETTE_ENTRIES.find((entry) => entry.id === id) ?? null;
}

/** An entry's key, with the DJ's own keys applied; null when it has none. */
export function paletteChord(entry: PaletteEntry, overrides: KeyOverrides = {}): KeyChord | null {
  const key = entry.key;
  if (key === undefined) return null;
  let found: KeyChord | undefined;
  if (key.from === "binding") {
    found = overrides[key.id] ?? BINDINGS.find((row) => row.id === key.id)?.chord;
  } else if (key.from === "view") {
    const view = SHELL_VIEWS.find((candidate) => candidate.action === key.action);
    found = view ? shellViewChord(view.id, overrides) : undefined;
  } else {
    found = key.chord;
  }
  return found === undefined || found.key === "" ? null : found;
}

/**
 * The entries a query finds: every word typed must appear in the entry's
 * name, its menu or one of its other words. Names that start with what was
 * typed come first; otherwise the registry's order is kept.
 */
export function filterPalette(
  entries: readonly PaletteEntry[],
  query: string,
  translate: (text: string) => string = (text) => text,
): PaletteEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter((word) => word !== "");
  if (words.length === 0) return [...entries];
  const found = entries.filter((entry) => {
    const haystack = [
      translate(entry.label), entry.label, translate(entry.group), entry.group, ...(entry.keywords ?? []),
    ].join(" ").toLowerCase();
    return words.every((word) => haystack.includes(word));
  });
  const first = words.join(" ");
  const starts = (entry: PaletteEntry) =>
    translate(entry.label).toLowerCase().startsWith(first) || entry.label.toLowerCase().startsWith(first);
  return [...found.filter(starts), ...found.filter((entry) => !starts(entry))];
}

/** What the palette knows about the window when it opens. */
export interface PaletteContext {
  /** The view shell is drawn (`view.viewShell`). */
  shell: boolean;
  /** The Library is locked for editing (rekordbox, or Library Protection). */
  readOnly: boolean;
  /** The Track and Playlist items' states, as `menuBarEnabled` works them out. */
  enabled: Readonly<Record<string, boolean>>;
  /** Whether something on screen answers a command (`hasCommand`). */
  has: (action: Action) => boolean;
}

/** Whether an entry can run now, and if not, why, in words for the DJ. */
export type PaletteAvailability = { available: true } | { available: false; why: string };

const BAR_MENUS = new Set(["Track", "Playlist"]);

export function paletteAvailability(entry: PaletteEntry, context: PaletteContext): PaletteAvailability {
  if (entry.oldLayoutOnly === true && context.shell) {
    return { available: false, why: "Not in the view shell yet" };
  }
  if (entry.writes === true && context.readOnly) {
    return { available: false, why: "The Library is locked for editing" };
  }
  if (entry.run.kind === "menu" && BAR_MENUS.has(entry.group) && entry.id !== "locate-loaded") {
    if (context.enabled[entry.run.id] !== true) {
      return {
        available: false,
        why: entry.group === "Track" ? "Select tracks in a list first" : "Select a playlist in the tree first",
      };
    }
  }
  if (entry.run.kind === "command" && !context.has(entry.run.action)) {
    return {
      available: false,
      why: entry.group === "Views" ? "Only in the view shell" : "Not on screen now",
    };
  }
  return { available: true };
}

/**
 * What a key press asks of the palette: `"open"` for its own key, an
 * entry's id for a key only the palette answers, or null. The entries' keys
 * are answered only where there is no native menu row to answer them
 * (Linux); on macOS and Windows the menu's accelerators still do.
 */
export function paletteKey(
  pressed: KeyChord,
  platform: Platform,
  overrides: KeyOverrides = {},
): "open" | string | null {
  if (platform.mac ? pressed.ctrlKey === true : pressed.metaKey === true) return null;
  if (chordMatches(PALETTE_OPEN_CHORD, pressed, platform)) return "open";
  if (platform.mac || platform.linux !== true) return null;
  for (const entry of PALETTE_ENTRIES) {
    const key = entry.key;
    if (key === undefined || key.from === "view" || key.handled !== "palette") continue;
    const bound = paletteChord(entry, overrides);
    if (bound !== null && chordMatches(bound, pressed, platform)) return entry.id;
  }
  return null;
}
