/**
 * The compact player's pure parts (ticket #51): the beat jump sizes its
 * selector offers and the position readout.
 *
 * The compact player is the view shell's deck, drawn by `CompactPlayer` from
 * the same `Player` that owns the engine. Its jump size is not the old deck's
 * menu: it offers four sizes only, and the choice is a stored preference
 * (`view.shellJumpBeats`) rather than the deck's session state, so it
 * persists. The deck's jump commands (the arrows) follow it.
 */
import { JUMP_SIZES } from "./player";

/** The sizes on the selector, in beats, in order. */
export const COMPACT_JUMP_BEATS = [4, 8, 16, 32] as const;
export type CompactJumpBeats = (typeof COMPACT_JUMP_BEATS)[number];

/** What the selector shows before anyone touches it: the design's render has 16 lit. */
export const COMPACT_JUMP_DEFAULT: CompactJumpBeats = 16;

/** A stored size, or the default when the value is not one of the four. */
export function compactJumpBeats(stored: unknown): CompactJumpBeats {
  return (COMPACT_JUMP_BEATS as readonly unknown[]).includes(stored)
    ? (stored as CompactJumpBeats)
    : COMPACT_JUMP_DEFAULT;
}

/** The deck's jump size (`JUMP_SIZES`) of that many beats: 32 is rekordbox's "8Bars". */
export function compactJumpSizeId(beats: CompactJumpBeats): string {
  const size = JUMP_SIZES.find((candidate) => candidate.beats === beats);
  if (!size) throw new Error(`no jump size of ${beats} beats`);
  return size.id;
}

function wholeSeconds(seconds: number): number {
  return Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
}

function minutesAndSeconds(whole: number): string {
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
}

/**
 * Elapsed time as the readout shows it: `1:23.4`, and `-0:01.9` in the
 * pre-roll a jump back from the start lands in (the engine plays silence
 * there until the head reaches zero), so the readout never says the start
 * while the deck is somewhere else.
 */
export function compactPosition(seconds: number): string {
  if (!Number.isFinite(seconds)) return "0:00.0";
  const tenths = Math.floor(Math.abs(seconds) * 10 + 1e-6);
  const sign = seconds < 0 && tenths > 0 ? "-" : "";
  return `${sign}${minutesAndSeconds(Math.floor(tenths / 10))}.${tenths % 10}`;
}

/** The track's length as the readout shows it: `5:48`. */
export function compactLength(seconds: number): string {
  return minutesAndSeconds(Math.floor(wholeSeconds(seconds)));
}
