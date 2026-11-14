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
import type { BrowserSide } from "./browserFocus";
import type { CommandScope } from "./commands";
import { menuBarItem } from "./menuBar";
import { SHELL_VIEWS, shellViewChord } from "./shellViews";
import { TARGET_ACTIONS } from "./targetList";
import { GO_TO_TREE } from "./shellTreeOverlay";
import {
  BINDINGS, chordMatches, type Action, type Binding, type KeyChord, type KeyOverrides, type Platform,
} from "./shortcuts";
import { INTAKE_ACTIONS } from "./intakeEdits";

/** The items the shell does itself (`SHELL_ITEMS` in `menu.rs`). */
export type ShellItem = "fullscreen" | "close-window" | "quit";

/** What running an entry does. */
export type PaletteRun =
  /** The window's own menu handler, with a native item's id. */
  | { kind: "menu"; id: string }
  /** The shell, for an item the window never heard of. */
  | { kind: "shell"; id: ShellItem }
  /**
   * A registered command (`commands.ts`), asked of the scope it was
   * registered for: a deck's, the list or tree the DJ was on (`browser`),
   * or the window's (none).
   */
  | { kind: "command"; action: Action; scope?: "a" | "b" | "browser" }
  /** Show the view shell, or upstream's old layout. */
  | { kind: "layout"; to: "shell" | "old" }
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
  /** The command it runs by another way (a menu item), so the key map's row for it is this entry. */
  action?: Action;
  /** Changes the Library: unavailable while it is locked. */
  writes?: boolean;
  /**
   * Acts on a part of upstream's layout the view shell does not draw: run
   * from the shell, it switches to the old layout first.
   */
  oldLayoutOnly?: boolean;
  /** What must be true for it to do anything: history to undo or redo, a loaded track. */
  needs?: "undo" | "redo" | "loaded";
}

const menu = (id: string): PaletteRun => ({ kind: "menu", id });
const binding = (id: string, handled: "window" | "palette" = "window"): PaletteKey => ({ from: "binding", id, handled });
const chord = (value: KeyChord, handled: "window" | "palette"): PaletteKey => ({ from: "chord", chord: value, handled });

/**
 * The attribute on the palette's own surface. A part of the window that
 * gives up the keyboard when the focus or a press goes elsewhere (the
 * compact player's arming) ignores the palette, which is only passing
 * through: closing it gives the keyboard back as it was.
 */
export const PALETTE_SURFACE = "data-command-palette";

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
  const bar = menuBarItem(id);
  // Analyze Track writes the analysis to the Library; the command refuses a
  // locked Library itself, so the menu item does not, but the palette greys it.
  const action: Action | undefined = id === "analyze-track" ? "analyseSelection" : bar?.action;
  const writes = id === "analyze-track" ? true : bar?.writes === true;
  const row = BINDINGS.find((candidate) => candidate.action !== undefined && candidate.action === action);
  return {
    id, label, group, run: menu(id), menu: id,
    ...(action === undefined ? {} : { action }),
    ...(writes ? { writes: true } : {}),
    ...(row ? { key: binding(row.id) } : {}),
  };
}

/** The entries written out: the views, the native menus' items, and the panes of Preferences. */
const EXPLICIT_ENTRIES: readonly PaletteEntry[] = [
  ...SHELL_VIEWS.map((view): PaletteEntry => ({
    id: `view-${view.id}`,
    label: `Show ${view.label}`,
    group: "Views",
    keywords: ["view", "tab"],
    run: { kind: "command", action: view.action },
    key: { from: "view", action: view.action },
  })),
  // The tree: over the view when it is too narrow to draw it, else focused (wbx-45.21).
  {
    id: "go-to-tree",
    label: GO_TO_TREE.label,
    group: "Views",
    keywords: ["tree", "playlist", "open", "breadcrumb", "folder"],
    run: { kind: "command", action: GO_TO_TREE.action },
    key: chord(GO_TO_TREE.chord, "window"),
  },
  // The Library view's second list (wbx-59): registered by the shell while
  // the Library view shows, so greyed elsewhere.
  ...TARGET_ACTIONS.map((entry): PaletteEntry => ({
    id: `target-${entry.action}`,
    label: entry.label,
    group: "Library",
    keywords: ["second list", "target"],
    run: { kind: "command", action: entry.action },
    key: chord(entry.chord, "window"),
    ...(entry.action === "toggleTargetList" || entry.action === "swapTargetLists" ? {} : { writes: true }),
  })),

  // The Intake view's actions (wbx-63): live while the view shows.
  ...INTAKE_ACTIONS.map((entry): PaletteEntry => ({
    id: `intake-${entry.action.slice("intake".length).toLowerCase()}`,
    label: entry.label,
    group: "Intake",
    keywords: ["intake", "prep", ...(entry.action.startsWith("intakeEnergy") ? ["energy", "rating"] : [])],
    run: { kind: "command", action: entry.action },
    key: chord(entry.chord, "window"),
    ...(entry.writes ? { writes: true } : {}),
  })),

  // The large player's Stems control (wbx-67): live while it offers a split.
  {
    id: "intake-splitstems", label: "Split Stems", group: "Intake", keywords: ["stems", "separate", "split", "vocals"],
    run: { kind: "command", action: "intakeSplitStems" },
  },

  // Start Player A unless it already plays; never pauses (wbx-74: the
  // Review view's Listen runs it after loading a row's track).
  {
    id: "player-a-play", label: "Play (never pause)", group: "Player A", keywords: ["start", "listen", "deck", "player 1"],
    run: { kind: "command", action: "deckPlayIfStopped", scope: "a" },
  },

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
    id: "undo", label: "Undo", group: "Edit", run: menu("undo"), menu: "undo", needs: "undo",
    key: chord({ key: "z", metaKey: true }, "window"),
  },
  {
    id: "redo", label: "Redo", group: "Edit", run: menu("redo"), menu: "redo", needs: "redo",
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
    menu: "predefined:select_all", key: binding("selectAll"), action: "selectAll",
  },

  // Track and Playlist.
  ...TRACK_ITEMS.map((item) => barEntry("Track", item)),
  {
    id: "locate-loaded", label: "Locate track loaded on deck", group: "Track",
    run: menu("locate-loaded"), menu: "locate-loaded", key: binding("locateLoaded"), action: "locateLoaded",
    needs: "loaded",
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
    id: "use-view-shell", label: "Use the view shell", group: "View", keywords: ["layout", "views", "new"],
    run: { kind: "layout", to: "shell" },
  },
  {
    id: "use-old-layout", label: "Use the old layout", group: "View",
    keywords: ["layout", "two-deck", "decks", "classic", "back"],
    run: { kind: "layout", to: "old" },
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
    // Unlock or lock a Stick (wbx-45.11): the window's own dialog.
    id: "stick-protection", label: "Stick Protection", group: "Devices", keywords: ["unlock", "lock", "usb", "stick", "protect"],
    run: { kind: "menu", id: "stick-protection" },
  },
  {
    // Its switch is on Advanced › Browse, a tab away from where Advanced
    // opens; this lands on it (wbx-45.15).
    id: "library-protection", label: "Library Protection", group: "Preferences", keywords: ["protect", "lock", "unlock", "read-only", "edit"],
    run: { kind: "preferences", pane: "libraryProtection" },
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

/** The commands the explicit entries already run, so the key map's rows for them are not listed twice. */
const COVERED: ReadonlySet<Action> = new Set(EXPLICIT_ENTRIES.flatMap((entry) => [
  ...(entry.run.kind === "command" ? [entry.run.action] : []),
  ...(entry.action === undefined ? [] : [entry.action]),
]));

/** Which registration a key map row's command is asked of (see `useCommands` in each part). */
function rowScope(row: Binding): "a" | "b" | "browser" | undefined {
  if (row.deck !== undefined) return row.deck;
  // The decks register the load commands, each its own (`Player.tsx`).
  if (row.action === "loadPlayer1") return "a";
  if (row.action === "loadPlayer2") return "b";
  // The lists register their movement and selection per browser side
  // (`TrackTable.tsx`); Clear Search is the window's.
  if (row.group === "Browse" && row.action !== "clearSearch") return "browser";
  return undefined;
}

/**
 * Every other command the window registers, one entry per row of the key
 * map: the decks' (Play/Pause, Cue, the jumps, the pads...), the lists'
 * movement, the master's volume and Mute. They are the very rows the key
 * handlers dispatch (`BINDINGS`), asked of the same registry (`runCommand`),
 * so a key and its palette entry cannot drift apart.
 */
const KEY_MAP_ENTRIES: readonly PaletteEntry[] = BINDINGS.flatMap((row): PaletteEntry[] => {
  const action = row.action;
  if (action === undefined || row.alias === true) return [];
  const scope = rowScope(row);
  if (row.deck === undefined && COVERED.has(action)) return [];
  const deckWords = row.deck === undefined ? [] : ["deck", "player", row.deck === "a" ? "player 1" : "player 2"];
  return [{
    id: `key-${row.id}`,
    label: row.label,
    group: row.group,
    ...(deckWords.length > 0 ? { keywords: deckWords } : {}),
    run: scope === undefined ? { kind: "command", action } : { kind: "command", action, scope },
    key: binding(row.id),
  }];
});

/**
 * Every entry, in the order the palette lists them before anything is
 * typed: the views, then the native menus in their order, then the parts of
 * Preferences the four views do not show, then the rest of the key map.
 */
export const PALETTE_ENTRIES: readonly PaletteEntry[] = [...EXPLICIT_ENTRIES, ...KEY_MAP_ENTRIES];

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

/** An entry's searchable text, lower-cased once per language. */
interface Prepared {
  /** Its name, translated and in English. */
  names: readonly string[];
  /** Its menu or group and its other words. */
  others: readonly string[];
}

const prepared = new WeakMap<(text: string) => string, Map<PaletteEntry, Prepared>>();

function prepare(entry: PaletteEntry, translate: (text: string) => string): Prepared {
  let cache = prepared.get(translate);
  if (cache === undefined) {
    cache = new Map();
    prepared.set(translate, cache);
  }
  let found = cache.get(entry);
  if (found === undefined) {
    const lower = (texts: readonly string[]) => [...new Set(texts.map((text) => text.toLowerCase()))];
    found = {
      names: lower([translate(entry.label), entry.label]),
      others: lower([translate(entry.group), entry.group, ...(entry.keywords ?? [])]),
    };
    cache.set(entry, found);
  }
  return found;
}

/**
 * How well one typed word matches a text: the start of it, the start of one
 * of its words, anywhere in it, or its letters in order with gaps (fuzzy,
 * so "prefs" finds Preferences). Zero when it does not match.
 */
export function wordScore(word: string, text: string): number {
  if (text.startsWith(word)) return 100;
  const at = text.indexOf(word);
  if (at > 0) return /[\s\-/(.]/.test(text[at - 1] ?? "") ? 80 : 60;
  // Letters in order: fewer and shorter gaps, and a first letter that
  // starts a word, rank higher.
  let from = 0;
  let gaps = 0;
  let first = -1;
  for (const letter of word) {
    const next = text.indexOf(letter, from);
    if (next < 0) return 0;
    if (first < 0) first = next;
    else if (next > from) gaps += 1;
    from = next + 1;
  }
  const startsWord = first === 0 || /[\s\-/(.]/.test(text[first - 1] ?? "");
  return Math.max(1, 40 - gaps * 4 + (startsWord ? 5 : 0));
}

/**
 * The entries a query finds, best first: every word typed must match the
 * entry's name (fuzzily) or its menu or other words (as a word or part of
 * one). Ties keep the registry's order.
 */
export function filterPalette(
  entries: readonly PaletteEntry[],
  query: string,
  translate: (text: string) => string = (text) => text,
): PaletteEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter((word) => word !== "");
  if (words.length === 0) return [...entries];
  const scored: { entry: PaletteEntry; score: number; index: number }[] = [];
  entries.forEach((entry, index) => {
    const { names, others } = prepare(entry, translate);
    let score = 0;
    for (const word of words) {
      let best = 0;
      for (const name of names) best = Math.max(best, wordScore(word, name));
      // A menu's or another word's letters scattered are too loose a match.
      for (const other of others) {
        const found = wordScore(word, other);
        if (found >= 60) best = Math.max(best, found / 2);
      }
      if (best === 0) return;
      score += best;
    }
    scored.push({ entry, score, index });
  });
  scored.sort((a, b) => b.score - a.score || a.index - b.index);
  return scored.map((found) => found.entry);
}

/** What the palette knows about the window when it opens. */
export interface PaletteContext {
  /** The view shell is drawn (`view.viewShell`). */
  shell: boolean;
  /** The Library is locked for editing (rekordbox, or Library Protection). */
  readOnly: boolean;
  /** The Track and Playlist items' states, as `menuBarEnabled` works them out. */
  enabled: Readonly<Record<string, boolean>>;
  /** Whether something on screen answers a command (`hasCommand`), for a scope. */
  has: (action: Action, scope?: CommandScope) => boolean;
  /** The browser side the DJ was on (the list or tree commands' scope). */
  side: BrowserSide;
  /** Whether Undo and Redo have anything to act on (a text field's own history counts). */
  history: { undo: boolean; redo: boolean };
  /** Whether each deck holds a track. */
  loaded: { a: boolean; b: boolean };
}

/** Whether an entry can run now, and if not, why, in words for the DJ. */
export type PaletteAvailability =
  | { available: true; note?: string }
  | { available: false; why: string };

/** A deck's commands that do something with no track loaded. */
const DECK_IDLE: ReadonlySet<Action> = new Set<Action>([
  "loadPlayer1", "loadPlayer2", "metronomeSound", "showMemory", "showHotCues", "showInfo",
  "eqKillLow", "eqKillMid", "eqKillHigh",
]);

const BAR_MENUS = new Set(["Track", "Playlist"]);

export function paletteAvailability(entry: PaletteEntry, context: PaletteContext): PaletteAvailability {
  const run = entry.run;
  if (run.kind === "layout") {
    const here = run.to === "shell" ? context.shell : !context.shell;
    return here
      ? { available: false, why: run.to === "shell" ? "Already showing the view shell" : "Already showing the old layout" }
      : { available: true };
  }
  if (entry.writes === true && context.readOnly) {
    return { available: false, why: "The Library is locked for editing" };
  }
  if (run.kind === "menu" && BAR_MENUS.has(entry.group) && entry.id !== "locate-loaded") {
    if (context.enabled[run.id] !== true) {
      return {
        available: false,
        why: entry.group === "Track" ? "Select tracks in a list first" : "Select a playlist in the tree first",
      };
    }
  }
  if (entry.needs === "undo" && !context.history.undo) return { available: false, why: "Nothing to undo" };
  if (entry.needs === "redo" && !context.history.redo) return { available: false, why: "Nothing to redo" };
  if (entry.needs === "loaded" && !context.loaded.a && !context.loaded.b) {
    return { available: false, why: "No track is loaded on a deck" };
  }
  if (run.kind === "command") {
    const scope = run.scope === "browser" ? context.side : run.scope;
    if (!context.has(run.action, scope)) {
      return {
        available: false,
        why: entry.group === "Views" ? "Only in the view shell" : run.scope === "b" ? "Only with two players" : "Not on screen now",
      };
    }
    if ((run.scope === "a" || run.scope === "b") && !context.loaded[run.scope] && !DECK_IDLE.has(run.action)) {
      return { available: false, why: `Load a track on Player ${run.scope.toUpperCase()} first` };
    }
  }
  if (entry.oldLayoutOnly === true && context.shell) {
    return { available: true, note: "Switches to the old layout" };
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
): string | null {
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
