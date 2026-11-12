/**
 * The view shell's four views and the keys that switch between them.
 *
 * The shell (`src/views/shell`) draws one frame (top bar, tree, player,
 * status line) and swaps the working area between these views. Each view is
 * a command (`viewLibrary` and so on in `shortcuts.ts`), so a tab, a key and
 * later the command palette all ask for it the same way.
 *
 * The keys are 1 to 4, which the Export key map also gives the deck: 1 to 3
 * are Set Hot Cue A to C and 4 is 1 Beat Loop. With the shell on the views
 * win: the shell reads the four keys here, ahead of the deck, and those four
 * deck commands are left to their pads and buttons. With the shell off
 * nothing reads this table, and the deck keeps its keys.
 *
 * The views are deliberately not rows of `BINDINGS`: that table is one key
 * map in which no two rows share a key (`keyPresets.test.ts`), and it is the
 * old layout's until ticket #77 removes that layout. A key stored against a
 * view's action in the Keyboard preferences is honoured all the same, so the
 * rows can move into the table, or the command palette's registry (#54),
 * without anyone's keys changing.
 */
import {
  chordMatches, isTyping, type Action, type FocusTarget, type KeyChord, type KeyOverrides, type Platform,
} from "./shortcuts";

export type ShellView = "library" | "intake" | "review" | "stick";

export interface ShellViewInfo {
  id: ShellView;
  /** The tab's name, in English; drawn through the translation. */
  label: string;
  /** The command that shows it, and the id a changed key is stored against. */
  action: Action;
  /** The key it is on unless the DJ changed it. */
  chord: KeyChord;
}

/** The views, in tab order: key 1 is the first. */
export const SHELL_VIEWS: readonly ShellViewInfo[] = [
  { id: "library", label: "Library", action: "viewLibrary", chord: { key: "1" } },
  { id: "intake", label: "Intake", action: "viewIntake", chord: { key: "2" } },
  { id: "review", label: "Review", action: "viewReview", chord: { key: "3" } },
  { id: "stick", label: "Stick", action: "viewStick", chord: { key: "4" } },
];

/**
 * The breakpoint from `docs/design/tokens.md`: at this width of the window
 * and below, in CSS pixels, the tree gives way. The layout itself is a
 * container query in `ViewShell.module.css`; this names the number for
 * tests and for anything that must agree with it.
 */
export const SHELL_NARROW_MAX_PX = 800;

/** The view the window opens on. */
export const DEFAULT_SHELL_VIEW: ShellView = "library";

/** The key a view is on, with the DJ's own applied; its `key` is empty when it has none. */
export function shellViewChord(view: ShellView, overrides: KeyOverrides = {}): KeyChord {
  const entry = SHELL_VIEWS.find((candidate) => candidate.id === view);
  return (entry && overrides[entry.action]) ?? entry?.chord ?? { key: "" };
}

/**
 * The view a key press asks for, or `null` when the press is not a view's:
 * some other key, a field or a dialog has the keyboard, or the other
 * platform's modifier is held.
 */
export function shellViewForKey(
  chord: KeyChord,
  platform: Platform,
  target: FocusTarget | null | undefined,
  overrides: KeyOverrides = {},
): ShellView | null {
  if (isTyping(target)) return null;
  // A dialog over the window owns its keys; the view under it stays put.
  // A confirmation prompt is an alertdialog (`ConfirmHost` and its kin).
  if (target?.closest?.('[role="dialog"], [role="alertdialog"], dialog')) return null;
  if (platform.mac ? chord.ctrlKey === true : chord.metaKey === true) return null;
  for (const view of SHELL_VIEWS) {
    if (chordMatches(shellViewChord(view.id, overrides), chord, platform)) return view.id;
  }
  return null;
}
