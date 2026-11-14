/**
 * "Save as intelligent playlist" from the Track Filter bar (wbx-58): what
 * the window sends `save_filter_playlist`, and the name it suggests.
 *
 * The filter goes in the Library service's terms (`toServiceSelection`), so
 * the backend resolves keys, colours and MyTags exactly as the bar's list
 * and `tracks.query` do. Two things the service form would lose are sent
 * apart: the open list when it is not All Tracks (an intelligent playlist
 * chooses from the whole Library), and whether Key match or BPM ±6% is on
 * (they follow the loaded track, which a rule cannot). With `NO_TRACK`,
 * `toServiceSelection` leaves those two out, so nothing of the loaded track
 * is frozen into the rule. The backend decides what an intelligent playlist
 * can hold and says why when it cannot (`src-tauri/src/filter_playlist.rs`).
 *
 * Pure.
 */
import type { SaveFilterRequest } from "@/ipc/types";
import { NO_TRACK, pickSummaries, toServiceSelection, type ShellFilter } from "@/lib/shellTrackFilter";

export function toSaveRequest(f: ShellFilter, within: string | null): SaveFilterRequest {
  const out = toServiceSelection(f, NO_TRACK) as SaveFilterRequest;
  if (within !== null) out.within = within;
  const follows: ("key" | "bpm")[] = [];
  if (f.keyMatch) follows.push("key");
  if (f.bpmMatch) follows.push("bpm");
  if (follows.length > 0) out.follows = follows;
  return out;
}

/** A name that says what is picked, as the bar's controls say it. */
export function suggestedPlaylistName(f: ShellFilter, keyName: (identity: string) => string): string {
  const said = pickSummaries(f, NO_TRACK, keyName);
  const parts = [
    ...Object.values(said.myTags),
    said.energy === null ? null : `Energy ${said.energy}`,
    said.prepColours === null ? null : `Prep ${said.prepColours}`,
    said.bpm === null ? null : `BPM ${said.bpm}`,
    said.keys,
  ].filter((part): part is string => part !== null && part !== "");
  const name = parts.join(" · ");
  return name === "" ? "Saved filter" : name.length > 80 ? `${name.slice(0, 79)}…` : name;
}
