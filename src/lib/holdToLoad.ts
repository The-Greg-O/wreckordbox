/**
 * The Stick load as a small state machine (wbx-72), pure so every step is
 * unit tested: idle, holding (the big button or `L` pressed and not yet
 * held long enough), loading (full, or shrunk to the status line), finished
 * (everything planned was written: safe to remove) and failed (refused, or
 * the load's report says what was and was not written).
 *
 * The hold: a press starts it, only that same input letting go ends it,
 * and it is full after `holdMs(kind)`: about one second for a Spare stick,
 * two for a Gig stick (docs/design/screens/stick). Released before it is
 * full, or interrupted (the window losing focus), nothing happens. A full
 * hold moves to `loading`; the shell then asks for the hold's approval of
 * exactly the plan shown and runs the load (`src/store/stickLoads.ts`).
 *
 * Progress comes from the service's `stick_load_progress` events
 * (`fromServiceProgress`); the store coalesces a burst before it gets here.
 */
import type { LoadOutcome } from "./stickLoad";
import { stickHoldMs } from "./stickWrites";

/** The input holding the button: a pointer by id, or a key. */
export type Holder = { pointer: number } | { key: string };

export interface LoadProgress {
  /** The export's stages (`checking`, `copying`, `database`, `verifying`, `publishing`), then `native_stems`, then `done` or `failed`. */
  stage: string;
  done: number;
  total: number;
  /** The track being written; its id is null when two loaded tracks share its title. */
  track: { id: string | null; title: string } | null;
}

export type LoadMachine =
  | { state: "idle" }
  | { state: "holding"; holder: Holder; since: number; needs: number }
  | { state: "loading"; shrunk: boolean; progress: LoadProgress | null }
  | { state: "finished"; outcome: LoadOutcome }
  | { state: "failed"; outcome: LoadOutcome | null; error: string | null };

export type LoadEvent =
  | { type: "press"; holder: Holder; at: number; kind: "gig" | "spare" }
  | { type: "release"; holder: Holder }
  | { type: "interrupt" }
  | { type: "tick"; at: number }
  | { type: "progress"; progress: LoadProgress }
  | { type: "shrink" }
  | { type: "restore" }
  | { type: "finished"; outcome: LoadOutcome }
  | { type: "failed"; error: string }
  | { type: "dismiss" };

export const IDLE: LoadMachine = { state: "idle" };

/** How long the hold must last: a Gig stick needs the longer one. */
export function holdMs(kind: "gig" | "spare"): number {
  return stickHoldMs(kind === "spare");
}

function sameHolder(a: Holder, b: Holder): boolean {
  return "pointer" in a ? "pointer" in b && a.pointer === b.pointer : "key" in b && a.key === b.key;
}

export function loadStep(machine: LoadMachine, event: LoadEvent): LoadMachine {
  switch (machine.state) {
    case "idle":
      return event.type === "press"
        ? { state: "holding", holder: event.holder, since: event.at, needs: holdMs(event.kind) }
        : machine;
    case "holding":
      switch (event.type) {
        case "release":
          return sameHolder(machine.holder, event.holder) ? IDLE : machine;
        case "interrupt":
          return IDLE;
        case "tick":
          return event.at - machine.since >= machine.needs ? { state: "loading", shrunk: false, progress: null } : machine;
        default:
          return machine;
      }
    case "loading":
      switch (event.type) {
        case "progress":
          return { ...machine, progress: event.progress };
        case "shrink":
          return machine.shrunk ? machine : { ...machine, shrunk: true };
        case "restore":
          return machine.shrunk ? { ...machine, shrunk: false } : machine;
        case "finished":
          return event.outcome.outcome === "loaded"
            ? { state: "finished", outcome: event.outcome }
            : { state: "failed", outcome: event.outcome, error: null };
        case "failed":
          return { state: "failed", outcome: null, error: event.error };
        default:
          return machine;
      }
    case "finished":
    case "failed":
      return event.type === "dismiss" ? IDLE : machine;
  }
}

/** The service's `stick_load_progress` event, or null when it is not one. */
export function fromServiceProgress(data: unknown): { stick: string; progress: LoadProgress } | null {
  if (typeof data !== "object" || data === null) return null;
  const event = data as Record<string, unknown>;
  if (typeof event.stick !== "string" || typeof event.stage !== "string") return null;
  const count = (value: unknown) => (typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : 0);
  const track = event.track as Record<string, unknown> | null | undefined;
  return {
    stick: event.stick,
    progress: {
      stage: event.stage,
      done: count(event.done),
      total: count(event.total),
      track: track && typeof track.title === "string"
        ? { id: typeof track.id === "string" ? track.id : null, title: track.title }
        : null,
    },
  };
}

/** How far the load is, 0 to 100; null while that is not known. */
export function progressPercent(progress: LoadProgress | null): number | null {
  if (progress === null) return null;
  if (progress.stage === "done") return 100;
  if (progress.total <= 0) return null;
  return Math.min(100, Math.round((100 * progress.done) / progress.total));
}

const STAGES: Record<string, string> = {
  checking: "Checking the Stick",
  copying: "Copying",
  database: "Writing the Stick's database",
  verifying: "Checking what was written",
  publishing: "Finishing the Stick's library",
  native_stems: "Writing Native stems",
  done: "Done",
  failed: "Stopped",
};

/** One plain line: the stage, how far, and the track being written. */
export function progressLine(progress: LoadProgress | null): string {
  if (progress === null) return "Starting…";
  const stage = STAGES[progress.stage] ?? progress.stage;
  const count = progress.total > 0 ? ` ${Math.min(progress.done, progress.total)} of ${progress.total}` : "";
  const track = progress.track !== null ? ` · ${progress.track.title}` : "";
  return `${stage}${count}${track}`;
}
