import type { Backend, DeviceFormat, DevicePlaylistEdit } from "@/ipc/types";

type DeviceDropBackend = Pick<Backend,
  "devicePlaylistCollectionTracks" | "devicePlaylistDuplicates" | "exportTracksToDevice">;

/**
 * Exports missing collection tracks as loose USB tracks, then returns the
 * device-library row ids to add to the dropped playlist. The caller applies
 * the edit through `useDeviceLibraries` so its cached tree is refreshed.
 */
export async function prepareDevicePlaylistDrop(
  backend: DeviceDropBackend,
  path: string,
  format: DeviceFormat,
  playlist: string,
  ids: readonly string[],
  defaults: Parameters<Backend["exportTracksToDevice"]>[2],
  compatibility: Parameters<Backend["exportTracksToDevice"]>[3],
  askDuplicates: (held: number, total: number, name: string) => Promise<boolean | null>,
  name: string,
  /**
   * The staged write (wbx-45.4), asked once the duplicate question is
   * answered and before anything is written: the tracks to export to the
   * Stick first, how many go into the playlist and the duplicates choice.
   * Answers the approval both writes carry, or null to stop, having
   * written nothing.
   */
  approve: (missing: readonly string[], adding: number, duplicates: boolean) => Promise<string | null>,
): Promise<{ edit: Extract<DevicePlaylistEdit, { kind: "add" }>; approval: string } | null> {
  if (ids.length === 0) return null;

  const before = await backend.devicePlaylistCollectionTracks(path, format, [...ids]);
  if (before.length !== ids.length) throw new Error("Only tracks of this library on the device can go into its playlists.");
  const existing = before.flatMap((row) => row === null ? [] : [row]);
  const held = await backend.devicePlaylistDuplicates(path, format, playlist, existing);
  const duplicates = await askDuplicates(held.length, ids.length, name);
  if (duplicates === null) return null;

  const missing = ids.filter((_, index) => before[index] === null);
  const adding = duplicates ? ids.length : ids.length - new Set(held).size;
  if (adding === 0) return null;
  const approval = await approve(missing, adding, duplicates);
  if (approval === null) return null;
  if (missing.length > 0) await backend.exportTracksToDevice([...missing], path, defaults, compatibility, approval);

  const after = await backend.devicePlaylistCollectionTracks(path, format, [...ids]);
  if (after.length !== ids.length || after.some((row) => row === null)) {
    throw new Error("Only tracks of this library on the device can go into its playlists.");
  }
  const heldSet = new Set(held);
  const tracks = (after as string[]).filter((row) => duplicates || !heldSet.has(row));
  return { edit: { kind: "add", playlist, tracks, duplicates }, approval };
}
