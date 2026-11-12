/**
 * The mock backend's cue history and test hooks for the Intake cue pads
 * (wbx-64).
 *
 * The shell records every cue edit as one entry of the shared Undo history
 * (`edit_cues` in `src-tauri/src/cues.rs`, `LibraryEdit::Cues`), so Edit ›
 * Undo takes a hot cue back off its pad. `recorded` does the same for the
 * mock: it snapshots the track's cues around the change and pushes an entry
 * whose Undo and Redo put the snapshot back and announce the track, as
 * `cues:changed` does.
 *
 * Hooks on `window`:
 * - `__hotCuesOf(id)`: the track's hot cues as the mock Library holds them,
 *   `{ letter, positionMs, colour }` in slot order.
 * - `__gridOf(id)`: the track's beat grid, `{ beatMs, bpmX100, locked }` (`beatMs`: the beat nearest 10 s),
 *   or `null` without one.
 * - `__failCueWrites(message | null)`: every cue write is refused with the
 *   message until it is cleared (a refused write, as the real Writer's).
 * - `__cueWrites()`: how many cue writes were asked of the mock.
 * - `__holdCueWrites()` / `__releaseCueWrites()`: cue writes wait, as a slow
 *   save (held before they change anything).
 * - `__holdCueReads(id | null)` / `__releaseCueReads()`: reads of a track's
 *   cues (one track, or every track with `null`) wait, as a slow read.
 * - `__holdGridWrites()` / `__releaseGridWrites()`, `__failGridWrites(message
 *   | null)`, `__gridWrites(kind?)`: the same for grid edits (counted by kind).
 * - `__gridBeatsOf(id)`: the whole grid; `__undoLabels()`: the shared
 *   history's Undo entries.
 */
import type { Cue } from "./types";

export interface MockCueSource {
  cuesOf: (track: string) => Cue[];
  /** Re-syncs the row's badges and tells the cue listeners, as `cues:changed`. */
  announce: (track: string) => void;
  /** Pushes one entry onto the mock's shared edit history. */
  record: (label: string, undo: () => void, redo: () => void) => void;
  grid: (track: string) => { beatMs: number; bpmX100: number; locked: boolean } | null;
  /** The track's whole grid, `[timeMs, number, tempoX100]` a beat. */
  beats: (track: string) => [number, number, number][] | null;
  /** The labels of the shared history's Undo entries, oldest first. */
  undoLabels: () => string[];
}

/** Calls that wait while held, all let go together. */
class Gate {
  private holding = false;
  private waiting: (() => void)[] = [];
  hold(): void { this.holding = true; }
  release(): void {
    this.holding = false;
    const waiting = this.waiting;
    this.waiting = [];
    for (const go of waiting) go();
  }
  pass(): Promise<void> {
    return this.holding ? new Promise<void>((resolve) => { this.waiting.push(resolve); }) : Promise.resolve();
  }
}

export function mockCueHooks(source: MockCueSource) {
  let failing: string | null = null;
  let failingGrid: string | null = null;
  let writes = 0;
  let gridWrites = 0;
  const gridKinds = new Map<string, number>();
  const cueWrites = new Gate();
  const cueReads = new Gate();
  let heldReadTrack: string | null = null;
  const gridGate = new Gate();
  const put = (track: string, cues: readonly Cue[]) => {
    const list = source.cuesOf(track);
    list.splice(0, list.length, ...cues.map((cue) => ({ ...cue })));
    source.announce(track);
  };
  if (typeof window !== "undefined") {
    const hooks = window as unknown as {
      __hotCuesOf: (id: string) => { letter: string; positionMs: number; colour: string | null }[];
      __gridOf: (id: string) => { beatMs: number; bpmX100: number; locked: boolean } | null;
      __failCueWrites: (message: string | null) => void;
      __cueWrites: () => number;
    };
    hooks.__hotCuesOf = (id) => source.cuesOf(id)
      .filter((cue) => !cue.memory)
      .sort((a, b) => a.letter.localeCompare(b.letter))
      .map((cue) => ({ letter: cue.letter, positionMs: cue.positionMs, colour: cue.colour }));
    hooks.__gridOf = (id) => source.grid(id);
    hooks.__failCueWrites = (message) => { failing = message; };
    hooks.__cueWrites = () => writes;
    const more = window as unknown as Record<string, unknown>;
    more.__holdCueWrites = () => cueWrites.hold();
    more.__releaseCueWrites = () => cueWrites.release();
    more.__holdCueReads = (track: string | null) => { heldReadTrack = track; cueReads.hold(); };
    more.__releaseCueReads = () => { heldReadTrack = null; cueReads.release(); };
    more.__holdGridWrites = () => gridGate.hold();
    more.__releaseGridWrites = () => gridGate.release();
    more.__failGridWrites = (message: string | null) => { failingGrid = message; };
    more.__gridWrites = (kind?: string) => (kind === undefined ? gridWrites : gridKinds.get(kind) ?? 0);
    more.__gridBeatsOf = (id: string) => source.beats(id);
    more.__undoLabels = () => source.undoLabels();
  }
  return {
    /** The refusal every cue write gets right now, or `null`; counts the write. */
    refusal: (): string | null => {
      writes += 1;
      return failing;
    },
    /** Waits while cue writes are held. */
    writeGate: (): Promise<void> => cueWrites.pass(),
    /** Waits while reads of `track`'s cues are held. */
    readGate: (track: string): Promise<void> =>
      (heldReadTrack === null || heldReadTrack === track ? cueReads.pass() : Promise.resolve()),
    /** Counts a grid write, waits while they are held, and answers its refusal or `null`. */
    gridWrite: async (kind: string): Promise<string | null> => {
      gridWrites += 1;
      gridKinds.set(kind, (gridKinds.get(kind) ?? 0) + 1);
      await gridGate.pass();
      return failingGrid;
    },
    /** Makes `change` to the track's cues as one recorded history entry. */
    recorded: (track: string, label: string, change: () => unknown): void => {
      const before = source.cuesOf(track).map((cue) => ({ ...cue }));
      change();
      const after = source.cuesOf(track).map((cue) => ({ ...cue }));
      source.record(label, () => put(track, before), () => put(track, after));
    },
  };
}
