# Backups

[Documentation](../README.md)

Preferences → Backups creates and lists library backups. Quit rekordbox before
creating one. Music files are excluded, so keep a separate backup of your audio.

## Contents and format

A new backup is a ZIP file named `rbexport-YYYYMMDD-HHMM.zip` using local
24-hour time. It contains:

- The library database, including playlists, tags, ratings, and history.
- The complete `PIONEER/USBANLZ` analysis folder, including cues, grids, waveforms, phrases, and vocals.
- `PIONEER/Artwork` images and thumbnails.
- Sync Manager selections and the Automix playlist, when present.
- `summary.json`, recording track/playlist/cue counts and component sizes.
- The Sidecar: what wreckordbox knows beside the Library. Its database
  (Facts, Runs, the record of which tracks have Stems, and whatever later
  versions keep there, such as Mutants) and the
  files you edit there (the kinds file, the Tag schema, Recipes).

Left out of the Sidecar's part, because it can be made again: audition
files, render caches and, unless you ask for it, Stem audio.

### Stem audio

Stem audio is not in a backup by default. It is about 215 MB for a
five-minute track (four lossless files), so 100 separated tracks add roughly
21 GB before compression, and lossless audio compresses little. The cost of
leaving it out is time: separating a track again takes over twenty minutes.

To include it, save a file named `backup-sidecar.json` holding
`{"stems": true}` in wreckordbox's backup state folder (on Linux
`~/.local/share/wreckordbox/backups/`). Every later backup then carries the
Stem audio, and says so in its `sidecar.json`.

Restoring a backup without Stem audio leaves any Stem audio already on the
machine where it is. A track whose Stem audio is not there afterwards has no
Stems and needs separating again; it is never shown as having them.

ZIP creation uses maximum Deflate compression and parallel work based on CPU
capacity. A same-minute backup is never overwritten. Backups stay until deleted.

## Storage and estimates

The saved-backup list shows date, time, and size. **Default backup folder →
Change folder…** changes where future backups are written and which folder
is listed. Existing archives remain in their original folders. The setting
persists across restarts.

The Rekordbox Data bar saves its last successful estimate. It refreshes a
week-old estimate on opening Backups or while that pane stays open.
**Refresh** requests an immediate estimate.

## Restore

Restore through the separate RBXport Restore app. Quit both RBXport and
rekordbox first. The restore app lists backups and uses `summary.json` when
available to show their contents.

Restore the whole backup or selected components: database, analysis, artwork,
or Sync Manager/Automix selections. The Sidecar is restored together with the
database, so what wreckordbox knows always matches the Library it is about.
A backup made before backups carried the Sidecar restores as it always did
and leaves the Sidecar on the machine untouched. If interrupted, the next RBXport or
RBXport Restore startup rolls back the operation, or completes it when new
files were already in place, before opening the library.

## Developer entry points

Backup integration lives in `src-tauri/src/backups.rs` and the adjacent
backup helper modules, with shared archive logic in `crates/rbl-backup/`. [How library backups work](../reference/backups.md)
describes the implementation and its design.
Validate backup/restore changes against disposable data, following [Testing](../development/testing.md).
