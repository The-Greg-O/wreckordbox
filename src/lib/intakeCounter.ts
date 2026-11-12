/**
 * The Intake counter (wbx-69): at half width the queue is "2 of 23" with
 * previous and next instead of a list. Where a step goes from the track
 * shown, on the page the view holds: another row of that page, or a read
 * of the neighbouring page's first (next) or last (previous) track, the
 * view then holding that page.
 */

/** The page the view holds and the row chosen on it (`-1`: none). */
export interface CounterPlace {
  /** Rows on the page. */
  length: number;
  /** The page's first row, counted in the whole queue. */
  offset: number;
  /** Where the next page starts; `null` on the last page. */
  nextOffset: number | null;
  index: number;
}

export type CounterStep =
  | { kind: "row"; at: number }
  /** Read the one track at `read`, then hold the page that starts at `offset`. */
  | { kind: "page"; read: number; offset: number };

/** Where previous (`-1`) or next (`1`) goes from `place`; `null` when it goes nowhere. */
export function counterStep(place: CounterPlace, delta: -1 | 1, pageSize = 200): CounterStep | null {
  const { length, offset, nextOffset, index } = place;
  if (length === 0) return null;
  if (index < 0) return delta === 1 ? { kind: "row", at: 0 } : null;
  const at = index + delta;
  if (at >= 0 && at < length) return { kind: "row", at };
  if (delta === 1) return nextOffset === null ? null : { kind: "page", read: nextOffset, offset: nextOffset };
  return offset === 0 ? null : { kind: "page", read: offset - 1, offset: Math.max(0, offset - pageSize) };
}
