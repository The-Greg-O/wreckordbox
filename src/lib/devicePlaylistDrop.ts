import type { Backend, DeviceFormat } from "@/ipc/types";

type DeviceDropBackend = Pick<Backend, "devicePlaylistCollectionTracks" | "devicePlaylistDuplicates">;

/** What a drop of Library tracks on a Stick playlist would do, as the DJ answered it. */
export interface DevicePlaylistDrop {
  /** Library tracks, in the order dropped. */
  tracks: string[];
  /** rekordbox's Add (true) or Skip answer about tracks the playlist holds. */
  duplicates: boolean;
  /** Entries the playlist gains. */
  adding: number;
  /** Tracks not on the Stick yet, exported to it first. */
  missing: number;
}

/**
 * The reads before a drop of Library tracks on a Stick playlist (wbx-45.4):
 * which tracks the Stick holds and which of them the playlist holds, then
 * rekordbox's duplicate question. Nothing is written: the write is one
 * approved backend call (`devicePlaylistAddLibraryTracks`), bound to these
 * exact tracks in this order and the answer, which exports the missing
 * ones and resolves every track to the Stick's own row itself.
 */
export async function askDevicePlaylistDrop(
  backend: DeviceDropBackend,
  path: string,
  format: DeviceFormat,
  playlist: string,
  ids: readonly string[],
  askDuplicates: (held: number, total: number, name: string) => Promise<boolean | null>,
  name: string,
): Promise<DevicePlaylistDrop | null> {
  if (ids.length === 0) return null;
  const tracks = [...ids];
  const before = await backend.devicePlaylistCollectionTracks(path, format, tracks);
  if (before.length !== tracks.length) throw new Error("Only tracks of this library on the device can go into its playlists.");
  const existing = before.flatMap((row) => row === null ? [] : [row]);
  const held = await backend.devicePlaylistDuplicates(path, format, playlist, existing);
  const duplicates = await askDuplicates(held.length, tracks.length, name);
  if (duplicates === null) return null;
  const adding = duplicates ? tracks.length : tracks.length - new Set(held).size;
  if (adding === 0) return null;
  return { tracks, duplicates, adding, missing: before.filter((row) => row === null).length };
}
