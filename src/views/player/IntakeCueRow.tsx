/**
 * The large player's control row (wbx-64): eight hot cue pads, then the
 * beat grid's nudge and tap tempo, under the detail waveform. The render is
 * `docs/design/screens/intake/intake-1280.png`: pads A to H with the cue's
 * time or "unset", then GRID ◀ 125.00 ▶ Tap. Beat jump is the strip's.
 *
 * Presentational: `Player` owns the deck and hands this its `useHotCues`
 * and `useGridEditor` results, so every edit is the deck's existing one,
 * and every one a step of the shared Undo history. Pads show letters; role
 * labels wait on the DJ's cue convention.
 *
 * Nothing acts until the loaded track's own cues (`ready`) and grid
 * (`grid.ready`) are read: until then the controls are off and say so, so a
 * press can never reach the previous track's cue. A slot whose write is on
 * its way says so and takes no second press.
 *
 * Keys. No new key is bound to the deck: every control here is a button
 * that owns its activation (the large player carries `data-own-activation`),
 * so Space and Enter press it and never reach the deck. A held key does not
 * repeat a pad, its ✕ or Tap (a repeated key down is dropped). The nudge
 * buttons are meant to repeat: a key moves the grid 1 ms a press and a
 * repeat, a pointer 1 ms a press and, held, 10 ms every 0.1 s after a
 * second; either way one hold is one Undo step. The palette has them all.
 */
import { memo, useEffect, useState } from "react";

import type { Cue } from "@/ipc/types";
import { formatBpm } from "@/lib/format";
import { HELD_SHIFT_MS, SHIFT_MS } from "@/lib/gridEdit";
import { HOLD_DELAY_MS, HOLD_REPEAT_MS, type HoldHandlers } from "./useHoldRepeat";
import { deckWrites, type TrackWriteFailure } from "@/lib/trackWrites";
import styles from "./IntakeCueRow.module.css";

export const INTAKE_PADS = ["A", "B", "C", "D", "E", "F", "G", "H"] as const;

/** `0:32.1`: minutes, seconds and tenths, as the render prints a pad's cue. */
export function padTime(ms: number): string {
  const tenths = Math.max(0, Math.floor(ms / 100));
  const minutes = Math.floor(tenths / 600);
  const seconds = Math.floor((tenths % 600) / 10);
  return `${minutes}:${String(seconds).padStart(2, "0")}.${tenths % 10}`;
}

/** What one press of a nudge moves, as its tooltip says. */
export const NUDGE_TITLE = (way: "earlier" | "later") =>
  `Shift the grid ${SHIFT_MS} ms ${way} a click or key press. Held: a key repeats ${SHIFT_MS} ms; `
  + `the pointer moves ${HELD_SHIFT_MS} ms every ${HOLD_REPEAT_MS / 1000} s after ${HOLD_DELAY_MS / 1000} s. One hold is one Undo.`;

/** The loaded track's failed cue and grid writes, kept current. */
export function useTrackWriteFailures(track: string | null): TrackWriteFailure[] {
  const [failures, setFailures] = useState<TrackWriteFailure[]>([]);
  useEffect(() => {
    const read = () => setFailures(track === null ? [] : deckWrites.failures().filter((f) => f.track === track));
    read();
    return deckWrites.subscribe(read);
  }, [track]);
  return failures;
}

export interface IntakeCueRowProps {
  /** The loaded track's cues are read: until then no pad acts. */
  ready: boolean;
  /** The hot cue in a slot, or `null`. */
  at: (letter: string) => Cue | null;
  /** What is on its way for a slot, or `null`. */
  pending: (letter: string) => "set" | "clear" | "move" | null;
  /** Sets an empty pad at the playhead, calls a set one. */
  press: (letter: string) => void;
  clear: (letter: string) => void;
  /** Whether a cue can be written; `editReason` says why not. */
  canEdit: boolean;
  editReason: string | undefined;
  /** The pad's colour, as the deck's markers draw it (`cueStyle`). */
  padStyle: (cue: Cue) => React.CSSProperties;
  grid: {
    /** The loaded track's grid is read: until then nothing here acts. */
    ready: boolean;
    bpmX100: number;
    tapping: boolean;
    canEdit: boolean;
    /** Why the grid buttons are off, when they are. */
    reason: string;
    earlier: HoldHandlers;
    later: HoldHandlers;
    /** A nudge by key: `repeat` keeps it in the hold the first press began. */
    key: (direction: -1 | 1, repeat: boolean) => void;
    tap: () => void;
  };
  /** The loaded track's cue and grid writes that failed. */
  failures: readonly TrackWriteFailure[];
}

/** A repeated key down presses nothing: the first one did. */
const noRepeat = (event: React.KeyboardEvent<HTMLButtonElement>) => {
  if (event.repeat && (event.key === "Enter" || event.key === " ")) event.preventDefault();
};

export const IntakeCueRow = memo(function IntakeCueRow({ ready, at, pending, press, clear, canEdit, editReason, padStyle, grid, failures }: IntakeCueRowProps) {
  const gridOn = grid.ready && grid.canEdit;
  // Enter and Space nudge on the key going down, each repeat too, as the
  // panel's Ctrl+arrow keys do; the click a key would make is not made.
  const nudgeKey = (direction: -1 | 1) => (event: React.KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (gridOn) grid.key(direction, event.repeat);
  };
  return (
    <div className={styles.row} data-testid="intake-cue-row">
      <div className={styles.pads} role="group" aria-label="Hot cue pads" data-ready={ready || undefined}>
        {INTAKE_PADS.map((letter) => {
          const cue = ready ? at(letter) : null;
          const onItsWay = ready ? pending(letter) : null;
          const title = !ready ? "Reading this track's cues"
            : onItsWay === "set" ? `Setting Hot Cue ${letter}`
              : onItsWay !== null ? `Saving Hot Cue ${letter}`
                : cue ? `Jump to Hot Cue ${letter} and play` : editReason ?? `Set Hot Cue ${letter} at the playhead`;
          return (
            <span key={letter} className={styles.slot}>
              <button
                type="button"
                className={styles.pad}
                data-set={cue ? "" : undefined}
                data-pending={onItsWay ?? undefined}
                style={cue ? padStyle(cue) : undefined}
                aria-label={`Hot cue ${letter}`}
                aria-busy={onItsWay !== null || undefined}
                title={title}
                disabled={!ready || onItsWay !== null || (!cue && !canEdit)}
                onKeyDown={noRepeat}
                onClick={() => press(letter)}
              >
                <b>{letter}</b>
                <span>{!ready ? "…" : onItsWay === "set" ? "setting" : cue ? padTime(cue.positionMs) : "unset"}</span>
              </button>
              {ready && cue && canEdit && cue.id !== "" && onItsWay === null ? (
                <button
                  type="button"
                  className={styles.clear}
                  aria-label={`Clear hot cue ${letter}`}
                  title={`Clear Hot Cue ${letter}`}
                  onKeyDown={noRepeat}
                  onClick={() => clear(letter)}
                >
                  ✕
                </button>
              ) : null}
            </span>
          );
        })}
      </div>
      <div className={styles.grid} role="group" aria-label="Beat grid" data-ready={grid.ready || undefined}>
        <span className={styles.label} aria-hidden>Grid</span>
        <button
          type="button"
          className={styles.button}
          aria-label="Shift the grid earlier"
          title={gridOn ? NUDGE_TITLE("earlier") : grid.reason}
          disabled={!gridOn}
          {...grid.earlier}
          onKeyDown={nudgeKey(-1)}
        >
          ◀
        </button>
        <span className={styles.bpm} data-testid="intake-grid-bpm" data-tapping={grid.tapping || undefined}>
          {grid.ready && grid.bpmX100 > 0 ? formatBpm(grid.bpmX100) : "—"}
        </span>
        <button
          type="button"
          className={styles.button}
          aria-label="Shift the grid later"
          title={gridOn ? NUDGE_TITLE("later") : grid.reason}
          disabled={!gridOn}
          {...grid.later}
          onKeyDown={nudgeKey(1)}
        >
          ▶
        </button>
        <button
          type="button"
          className={styles.button}
          aria-label="Tap the tempo"
          title={gridOn ? "Tap the tempo; it is applied from the second tap, and the run is one Undo" : grid.reason}
          disabled={!gridOn}
          onKeyDown={noRepeat}
          // A tap counts when the button goes down (see the GRID panel's
          // TAP); a click with no pointer behind it (the keyboard) taps too.
          onPointerDown={(event) => { if (event.button === 0) grid.tap(); }}
          onClick={(event) => { if (event.detail === 0) grid.tap(); }}
        >
          Tap
        </button>
      </div>
      {failures.length > 0 ? (
        <p className={styles.failed} data-testid="intake-pads-failed" role="status">
          {failures.map((f) => (
            <span key={f.kind}>
              Not saved: {f.kind === "cues" ? "cues" : "grid"}: {f.message}{" "}
              <button type="button" className={styles.dismiss} onClick={() => deckWrites.dismiss(f.track, f.kind)}>Dismiss</button>
            </span>
          ))}
        </p>
      ) : null}
    </div>
  );
});
