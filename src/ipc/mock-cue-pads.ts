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
 *   `{ letter, positionMs }` in slot order.
 * - `__gridOf(id)`: the track's beat grid, `{ beatMs, bpmX100, locked }` (`beatMs`: the beat nearest 10 s),
 *   or `null` without one.
 * - `__failCueWrites(message | null)`: every cue write is refused with the
 *   message until it is cleared (a refused write, as the real Writer's).
 * - `__cueWrites()`: how many cue writes were asked of the mock.
 */
import type { Cue } from "./types";

export interface MockCueSource {
  cuesOf: (track: string) => Cue[];
  /** Re-syncs the row's badges and tells the cue listeners, as `cues:changed`. */
  announce: (track: string) => void;
  /** Pushes one entry onto the mock's shared edit history. */
  record: (label: string, undo: () => void, redo: () => void) => void;
  grid: (track: string) => { beatMs: number; bpmX100: number; locked: boolean } | null;
}

export function mockCueHooks(source: MockCueSource) {
  let failing: string | null = null;
  let writes = 0;
  const put = (track: string, cues: readonly Cue[]) => {
    const list = source.cuesOf(track);
    list.splice(0, list.length, ...cues.map((cue) => ({ ...cue })));
    source.announce(track);
  };
  if (typeof window !== "undefined") {
    const hooks = window as unknown as {
      __hotCuesOf: (id: string) => { letter: string; positionMs: number }[];
      __gridOf: (id: string) => { beatMs: number; bpmX100: number; locked: boolean } | null;
      __failCueWrites: (message: string | null) => void;
      __cueWrites: () => number;
    };
    hooks.__hotCuesOf = (id) => source.cuesOf(id)
      .filter((cue) => !cue.memory)
      .sort((a, b) => a.letter.localeCompare(b.letter))
      .map((cue) => ({ letter: cue.letter, positionMs: cue.positionMs }));
    hooks.__gridOf = (id) => source.grid(id);
    hooks.__failCueWrites = (message) => { failing = message; };
    hooks.__cueWrites = () => writes;
  }
  return {
    /** The refusal every cue write gets right now, or `null`; counts the write. */
    refusal: (): string | null => {
      writes += 1;
      return failing;
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
