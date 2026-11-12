/**
 * The Intake view's edits (wbx-63), in the view's terms: the DJ's Prep
 * colours as the Library service names them, the view's actions and their
 * keys, and which queued track comes next.
 *
 * The rhythm of Intake is fix, mark, next. "Mark Clean and next" sets the
 * Prep colour the DJ named Clean and loads the next queued track; P marks
 * Problem and moves on; ] moves on without changing anything; Shift with 1
 * to 5 sets Energy (plain 1 to 4 are the views', `shellViews.ts`).
 *
 * The keys are the view's own while it shows. Like the view keys they are
 * read on the way down, ahead of the deck: in the Export key map P is a
 * deck's Loop Out or Call Next Memory Cue, and Shift with 1 to 4 is Player
 * B's Set Hot Cue A to D, a deck the view shell does not draw. With the
 * shell off, or another view showing, nothing reads this table. A key the
 * DJ stored against one of these actions in the Keyboard preferences is
 * honoured; an empty one unbinds it.
 */
import { chordMatches, isTyping, type Action, type FocusTarget, type KeyChord, type KeyOverrides, type Platform } from "./shortcuts";

/** A colour label of the Library with the DJ's name for it (the service's `PrepColour`). */
export interface PrepColour {
  /** 1 to 8, as the Library numbers its colour labels. */
  id: number;
  /** The colour itself: `Green`. */
  colour: string;
  /** What the DJ calls it: `Clean`. The colour's own name when not renamed. */
  label: string;
}

/** The DJ's Prep colours, and which mean Clean and Problem (`null`: none named so). */
export interface IntakeColours {
  labels: PrepColour[];
  clean: PrepColour | null;
  problem: PrepColour | null;
}

/** The service's `intake.colours` answer, as it arrives. */
export interface ServiceColours {
  colours: PrepColour[];
  clean: PrepColour | null;
  problem: PrepColour | null;
  problem_unavailable?: { code: string; message: string } | undefined;
}

export function fromServiceColours(raw: ServiceColours): IntakeColours {
  const lift = (colour: PrepColour | null) => (colour ? { id: colour.id, colour: colour.colour, label: colour.label } : null);
  return {
    labels: raw.colours.map((colour) => ({ id: colour.id, colour: colour.colour, label: colour.label })),
    clean: lift(raw.clean),
    problem: lift(raw.problem),
  };
}

export type IntakeAction =
  | "intakeMarkClean" | "intakeMarkProblem" | "intakeSkip"
  | "intakeEnergy1" | "intakeEnergy2" | "intakeEnergy3" | "intakeEnergy4" | "intakeEnergy5"
  | "intakeTag";

export interface IntakeActionInfo {
  action: IntakeAction & Action;
  /** Its name in the command palette, in English. */
  label: string;
  /** Its key unless the DJ changed it. `metaKey` is the platform's primary modifier (Ctrl on Linux). */
  chord: KeyChord;
  /** Changes the Library (greyed in the palette while it is locked). */
  writes: boolean;
}

const energy = (level: 1 | 2 | 3 | 4 | 5): IntakeActionInfo => ({
  action: `intakeEnergy${level}`,
  label: `Set Energy ${level}`,
  chord: { key: String(level), shiftKey: true },
  writes: true,
});

/** The view's actions, in the order the palette lists them. */
export const INTAKE_ACTIONS: readonly IntakeActionInfo[] = [
  { action: "intakeMarkClean", label: "Mark Clean and Next", chord: { key: "Enter", metaKey: true }, writes: true },
  { action: "intakeMarkProblem", label: "Mark Problem and Next", chord: { key: "p" }, writes: true },
  { action: "intakeSkip", label: "Skip to Next Track", chord: { key: "]" }, writes: false },
  energy(1), energy(2), energy(3), energy(4), energy(5),
  // The tag picker (wbx-65): any dimension's tag by search. Opening it
  // writes nothing; the click it makes goes through the edit path.
  { action: "intakeTag", label: "Tag Track…", chord: { key: "t" }, writes: false },
];

/** An action's key, with the DJ's own applied; its `key` is empty when it has none. */
export function intakeChord(action: IntakeAction, overrides: KeyOverrides = {}): KeyChord {
  const entry = INTAKE_ACTIONS.find((candidate) => candidate.action === action);
  return overrides[action] ?? entry?.chord ?? { key: "" };
}

/**
 * The action a key press asks of the Intake view, or `null`: another key,
 * a text field or a dialog has the keyboard.
 */
export function intakeActionForKey(
  chord: KeyChord,
  platform: Platform,
  target: FocusTarget | null | undefined,
  overrides: KeyOverrides = {},
): IntakeAction | null {
  if (isTyping(target)) return null;
  if (target?.closest?.('[role="dialog"], [role="alertdialog"], dialog')) return null;
  for (const entry of INTAKE_ACTIONS) {
    if (chordMatches(intakeChord(entry.action, overrides), chord, platform)) return entry.action;
  }
  return null;
}

/** The Energy an `intakeEnergyN` action sets, or `null` for another action. */
export function energyOf(action: IntakeAction): number | null {
  const level = /^intakeEnergy([1-5])$/.exec(action);
  return level ? Number(level[1]) : null;
}

/**
 * Where the next track is on a page of `length` rows, after the one at
 * `index` (`-1`: none chosen, so the first); `null` past the page's end.
 */
export function nextQueuedIndex(length: number, index: number): number | null {
  const next = index < 0 ? 0 : index + 1;
  return next < length ? next : null;
}
