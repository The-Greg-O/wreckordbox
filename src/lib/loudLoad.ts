/**
 * The loud load display's arithmetic (wbx-73), pure so it is unit tested:
 * how fast the display moves (one beat of the track being written), how
 * long its write head takes across a track (what recent tracks took, since
 * the service says only which track is being written, not how far into it),
 * the line under the track's title, and the shape drawn while a track has
 * no waveform of its own to show.
 *
 * The display itself is `src/views/shell/LoudLoad.tsx`.
 */
import { toCamelot } from "./camelot";

/** One beat of the track, in ms, folded into 85 to 175 BPM; 120 BPM when it has no tempo. */
export function beatMs(bpmX100: number | null): number {
  let bpm = bpmX100 !== null && Number.isFinite(bpmX100) && bpmX100 > 0 ? bpmX100 / 100 : 120;
  while (bpm < 85) bpm *= 2;
  while (bpm > 175) bpm /= 2;
  return 60_000 / bpm;
}

const SWEEP_GUESS_MS = 2000;
const SWEEP_LEAST_MS = 300;
const SWEEP_MOST_MS = 20_000;
/** How many of the latest tracks the guess follows. */
const SWEEP_RECENT = 5;

/**
 * How long the write head should take across the track being written: the
 * median time the last few tracks took (`took`, oldest first), two seconds
 * before any is known, kept between 0.3 and 20 seconds.
 */
export function sweepMs(took: readonly number[]): number {
  const recent = took.slice(-SWEEP_RECENT);
  if (recent.length === 0) return SWEEP_GUESS_MS;
  const sorted = [...recent].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  const median = sorted.length % 2 === 1 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
  return Math.min(SWEEP_MOST_MS, Math.max(SWEEP_LEAST_MS, median));
}

/** "Artist · 128 BPM · 8A", leaving out what the track does not have. */
export function nowDetail(details: { artist: string; bpmX100: number; key: string }): string {
  const parts: string[] = [];
  if (details.artist.trim() !== "") parts.push(details.artist.trim());
  if (details.bpmX100 > 0) parts.push(`${Math.round(details.bpmX100) / 100} BPM`);
  const key = details.key.trim() === "" ? "" : toCamelot(details.key) || details.key.trim();
  if (key !== "") parts.push(key);
  return parts.join(" · ");
}

/**
 * Three-band columns (low, mid, high, as the overview tag `PWV6` holds
 * them) for a track with no waveform to show: a made-up shape seeded by the
 * track, so each track still looks like its own, drawn by the display in a
 * muted colour so it is never taken for the track's real waveform.
 */
export function standInBands(seed: string, columns: number): Uint8Array {
  let state = 2166136261;
  for (let i = 0; i < seed.length; i += 1) state = Math.imul(state ^ seed.charCodeAt(i), 16777619) >>> 0;
  const random = () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const beats = 24 + Math.floor(random() * 40);
  const out = new Uint8Array(columns * 3);
  for (let column = 0; column < columns; column += 1) {
    const at = column / Math.max(1, columns);
    // Quiet intro and outro, a breakdown somewhere in the middle.
    const shape = Math.min(1, at * 6, (1 - at) * 6) * (Math.abs(at - 0.55) < 0.06 ? 0.45 : 1);
    const kick = 1 - ((at * beats) % 1);
    const low = 30 + 90 * shape * (0.5 + 0.5 * kick) * (0.8 + 0.2 * random());
    out[column * 3] = Math.max(8, Math.round(low));
    out[column * 3 + 1] = Math.max(4, Math.round(70 * shape * (0.6 + 0.4 * random())));
    out[column * 3 + 2] = Math.round(25 * shape * random());
  }
  return out;
}

/** The track the display draws, and since when (ms). */
export interface Shown { shown: string; since: number }

/**
 * Which track the display draws now that `latest` is being written: it
 * changes at most every `gap` ms, so a load racing through small or
 * already-written tracks costs a waveform read and redraw a few times a
 * second at most, never one per progress update. `retryIn` says when to
 * look again (the latest track then shown, if it is still the latest).
 */
export function nextShown(current: Shown, latest: string, now: number, gap: number): { show: Shown; retryIn: number | null } {
  if (latest === current.shown) return { show: current, retryIn: null };
  const wait = current.since + gap - now;
  if (wait > 0) return { show: current, retryIn: wait };
  return { show: { shown: latest, since: now }, retryIn: null };
}
