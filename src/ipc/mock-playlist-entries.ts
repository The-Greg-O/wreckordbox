/**
 * Playlist entries in the mock backend (wbx-59), as the Library keeps them:
 * one row per track in a playlist, with an identity of its own, so a track
 * that is in a playlist twice is two entries.
 *
 * The mock's membership stays a list of track ids per playlist (every other
 * edit changes that). The entries are kept beside it and matched to it
 * whenever it changed some other way: each track takes the first unused
 * entry of that track, in order, and a track new to the list a new entry.
 * The two reorders set the entries themselves:
 *
 * - by track (`reorderPlaylist`, upstream's writer): every row of a track
 *   gets the number of that track's last place in the order, and rows with
 *   one number are listed oldest first. So "move the second of two entries
 *   up" cannot be said: both move together, as the real writer does;
 * - by entry (`reorderPlaylistEntries`, wbx-59): exactly the order given,
 *   refused when it is not every entry of the playlist once.
 */
import type { PlaylistEntry } from "./types";

export interface MockEntry {
  id: string;
  track: string;
  /** The row's age: rows with one number are listed oldest first. */
  rowid: number;
}

export interface MockEntriesDeps {
  membersOf: (playlist: string) => string[];
  setMembers: (playlist: string, tracks: string[]) => void;
}

export function createMockEntries(deps: MockEntriesDeps) {
  const rows = new Map<string, MockEntry[]>();
  let next = 1;
  const make = (track: string): MockEntry => {
    const made = { id: `mock-entry-${String(next).padStart(5, "0")}`, track, rowid: next };
    next += 1;
    return made;
  };

  const entriesOf = (playlist: string): MockEntry[] => {
    const tracks = deps.membersOf(playlist);
    const held = rows.get(playlist) ?? [];
    if (held.length === tracks.length && held.every((entry, i) => entry.track === tracks[i])) return held;
    const unused = [...held];
    const matched = tracks.map((track) => {
      const at = unused.findIndex((entry) => entry.track === track);
      return at >= 0 ? unused.splice(at, 1)[0]! : make(track);
    });
    rows.set(playlist, matched);
    return matched;
  };

  const setEntries = (playlist: string, entries: readonly MockEntry[]) => {
    rows.set(playlist, [...entries]);
    deps.setMembers(playlist, entries.map((entry) => entry.track));
  };

  /** Upstream's `Writer::reorder`: by track, the last place of a track numbering all its rows. */
  const reorderByTrack = (playlist: string, order: readonly string[]): MockEntry[] => {
    const entries = entriesOf(playlist);
    const existing = entries.map((entry) => entry.track);
    const sequence = order.filter((track) => existing.includes(track));
    for (const track of existing) if (!sequence.includes(track)) sequence.push(track);
    const number = new Map<string, number>();
    sequence.forEach((track, i) => number.set(track, i + 1));
    const after = [...entries].sort((a, b) => (number.get(a.track)! - number.get(b.track)!) || a.rowid - b.rowid);
    setEntries(playlist, after);
    return after;
  };

  /** The order by entry, or why it is refused (it is not every entry once). */
  const orderByEntry = (playlist: string, ids: readonly string[]): MockEntry[] => {
    const entries = entriesOf(playlist);
    const named = [...ids].sort();
    const live = entries.map((entry) => entry.id).sort();
    if (named.length !== live.length || named.some((id, i) => id !== live[i])) {
      throw new Error("That could not be saved: the playlist's tracks have changed since.");
    }
    return ids.map((id) => entries.find((entry) => entry.id === id)!);
  };

  const wire = (playlist: string): PlaylistEntry[] =>
    entriesOf(playlist).map((entry) => ({ entryId: entry.id, trackId: entry.track }));

  return { entriesOf, setEntries, reorderByTrack, orderByEntry, wire };
}
