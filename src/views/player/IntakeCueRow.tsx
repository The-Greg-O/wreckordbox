/**
 * The large player's control row (wbx-64): eight hot cue pads, then the
 * beat grid's nudge and tap tempo, under the detail waveform. The render is
 * `docs/design/screens/intake/intake-1280.png`: pads A to H with the cue's
 * time or "unset", then GRID ◀ 125.00 ▶ Tap. Beat jump is the strip's.
 *
 * Presentational: `Player` owns the deck and hands this its `useHotCues`
 * and `useGridEditor` results, so every edit is the deck's existing one. A
 * cue goes through the backend's cue commands (one entry of the shared Undo
 * history each); the grid through the grid editor (its own Undo, which
 * Ctrl+Z reaches while the player has the keyboard).
 *
 * Pads show letters; role labels wait on the DJ's cue convention.
 *
 * Keys. No new key is bound: every control here is a button that owns its
 * activation (the large player carries `data-own-activation`), so Space and
 * Enter press it and never reach the deck. A held key does not repeat a pad,
 * its ✕ or Tap (a repeated key down is dropped); the nudge buttons are meant
 * to repeat, a millisecond a repeat, as the GRID panel's do.
 */
import { memo } from "react";

import type { Cue } from "@/ipc/types";
import { formatBpm } from "@/lib/format";
import styles from "./IntakeCueRow.module.css";
import type { HoldHandlers } from "./useHoldRepeat";

export const INTAKE_PADS = ["A", "B", "C", "D", "E", "F", "G", "H"] as const;

/** `0:32.1`: minutes, seconds and tenths, as the render prints a pad's cue. */
export function padTime(ms: number): string {
  const tenths = Math.max(0, Math.floor(ms / 100));
  const minutes = Math.floor(tenths / 600);
  const seconds = Math.floor((tenths % 600) / 10);
  return `${minutes}:${String(seconds).padStart(2, "0")}.${tenths % 10}`;
}

export interface IntakeCueRowProps {
  /** The hot cue in a slot, or `null`. */
  at: (letter: string) => Cue | null;
  /** Sets an empty pad at the playhead, calls a set one. */
  press: (letter: string) => void;
  clear: (letter: string) => void;
  /** Whether a cue can be written; `editReason` says why not. */
  canEdit: boolean;
  editReason: string | undefined;
  /** The pad's colour, as the deck's markers draw it (`cueStyle`). */
  padStyle: (cue: Cue) => React.CSSProperties;
  grid: {
    bpmX100: number;
    tapping: boolean;
    canEdit: boolean;
    /** Why the grid buttons are off, when they are. */
    reason: string;
    earlier: HoldHandlers;
    later: HoldHandlers;
    tap: () => void;
  };
  /** The last cue or grid write's failure, or `null`. */
  failure: string | null;
}

/** A repeated key down presses nothing: the first one did. */
const noRepeat = (event: React.KeyboardEvent<HTMLButtonElement>) => {
  if (event.repeat && (event.key === "Enter" || event.key === " ")) event.preventDefault();
};

export const IntakeCueRow = memo(function IntakeCueRow({ at, press, clear, canEdit, editReason, padStyle, grid, failure }: IntakeCueRowProps) {
  return (
    <div className={styles.row} data-testid="intake-cue-row">
      <div className={styles.pads} role="group" aria-label="Hot cue pads">
        {INTAKE_PADS.map((letter) => {
          const cue = at(letter);
          return (
            <span key={letter} className={styles.slot}>
              <button
                type="button"
                className={styles.pad}
                data-set={cue ? "" : undefined}
                style={cue ? padStyle(cue) : undefined}
                aria-label={`Hot cue ${letter}`}
                title={cue ? `Jump to Hot Cue ${letter}` : editReason ?? `Set Hot Cue ${letter} at the playhead`}
                disabled={!cue && !canEdit}
                onKeyDown={noRepeat}
                onClick={() => press(letter)}
              >
                <b>{letter}</b>
                <span>{cue ? padTime(cue.positionMs) : "unset"}</span>
              </button>
              {cue && canEdit && cue.id !== "" ? (
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
      <div className={styles.grid} role="group" aria-label="Beat grid">
        <span className={styles.label} aria-hidden>Grid</span>
        <button
          type="button"
          className={styles.button}
          aria-label="Shift the grid earlier"
          title={grid.canEdit ? "Shift the grid 1 ms earlier (hold to repeat)" : grid.reason}
          disabled={!grid.canEdit}
          {...grid.earlier}
        >
          ◀
        </button>
        <span className={styles.bpm} data-testid="intake-grid-bpm" data-tapping={grid.tapping || undefined}>
          {grid.bpmX100 > 0 ? formatBpm(grid.bpmX100) : "—"}
        </span>
        <button
          type="button"
          className={styles.button}
          aria-label="Shift the grid later"
          title={grid.canEdit ? "Shift the grid 1 ms later (hold to repeat)" : grid.reason}
          disabled={!grid.canEdit}
          {...grid.later}
        >
          ▶
        </button>
        <button
          type="button"
          className={styles.button}
          aria-label="Tap the tempo"
          title={grid.canEdit ? "Tap the tempo; it is applied from the second tap" : grid.reason}
          disabled={!grid.canEdit}
          onKeyDown={noRepeat}
          // A tap counts when the button goes down (see the GRID panel's
          // TAP); a click with no pointer behind it (the keyboard) taps too.
          onPointerDown={(event) => { if (event.button === 0) grid.tap(); }}
          onClick={(event) => { if (event.detail === 0) grid.tap(); }}
        >
          Tap
        </button>
      </div>
      {failure !== null ? (
        <p className={styles.failed} data-testid="intake-pads-failed" role="status">Not saved: {failure}</p>
      ) : null}
    </div>
  );
});
