# Library write paths and Ledger coverage

Audited for wbx-14.11, 2026-11-13. This is a coverage inventory, not a claim
that every recorded operation can be unwound. It covers production window,
service/CLI and scripting entry points, their row writes, and companion
Library/audio files. Tests, fixture builders and developer examples are not
production entry points. Stick exports, Sidecar-only changes and preferences
are distinguished below.

All ordinary row writes converge on `LibraryCore::write_then_recording` in
`crates/rbl-library/src/lib.rs`. File publication starts with `file_edit` or
runs inside an already guarded recorded row edit. The host installs the
Ledger on that core. Direct use of low-level `rbl_db::write::Writer` by a
standalone consumer has no Ledger; the Library service is the production
owner. Read requests do not settle intents, recover spills or publish files.

## Sources and recording shapes

- **Window** below means `dj:dj`, in the window's edit Run. The scripting
  bridge dispatches window commands and has that same attribution; it does
  not accept an agent source. This is different from the service protocol.
- **Caller** means the source on a supplied service Run or Run argument;
  otherwise `tool:wreckordbox`. Hosted `Service::call` without an attributed
  request uses the window/DJ context. Existing device/XML/iTunes interfaces
  are window commands; their hosted recorder seam also preserves an explicitly
  attributed caller when work moves to a worker thread.
- **Values** means the Ledger stores the history's cheap before/after values.
  `track`, `track_tags`, `my_tag_order` and `reorder_playlist_entries` have
  restorable payloads. `key_edit` retains native fields and audio key values,
  but is explicitly not reversible by Unwind. Other history kinds currently
  retain a name and links rather than a complete durable inverse.
- **Operation** means one bulk Run, bounded batches of 128 records / 64 KiB,
  with accepted outcomes generated after releasing native guards. Analysis
  and imports do not read audio or scan the Library merely to record it.
  Native analysis/import/tag-writing work still reads the audio it needs.
  Imported values never become DJ Facts simply because the DJ initiated them.

## Row and companion-file paths

| Entry points / implementation | Library mutation | Recorded? / shape | Source / Run |
|---|---|---|---|
| `commands::{set_track_rating,set_track_color,set_track_comment}`; `details::set_track_field` except BPM | Energy, Prep colour, comment and Info fields; lookup rows created by those setters | Yes, `track` values. Deliberate supported DJ setters also capture Facts/assertions; unchanged explicit Energy/colour choices are retained. | Window; service `tracks.rate` / `tracks.set_prep_colour` use Caller |
| `details::set_track_field` with key-tag preference | Native key plus MP3/AIFF/FLAC key tag | Yes, `key_edit`: track edits and `{path,before,after}` for each published file. Unsupported formats remain a native-only `track` edit. Quick Undo/Redo retain the files; Unwind does not restore them. | Window; same edit Run as native key |
| `key_tag::write_logged` host helper | Guarded audio key publication without a companion row | Yes, `audio_key_tag` file intent/entry with cheap values outside a bulk scope; an active operation owns its completion | Ambient Caller/window; production analysis uses the operation path below |
| `details::{set_my_tags,put_my_tag,write_my_tag_comments}` | Track-tag joins and optional comments | Yes, `track_tags` values; no entry if no write | Window |
| `details::{create_my_tag,rename_my_tag,delete_my_tag}` | Vocabulary rows, references/comments/rules affected by deletion | Yes, named `unrecorded` or `permanent` entry; no full durable inverse | Window |
| Service `mytag_edits::{create,rename,reorder,merge,delete,assign}` (assign also unassign); `tag_panel::set_my_tags` | Vocabulary/order/track-tag rows and optional comments | Yes: order/assign values; create/rename named outside-history; merge/delete named permanent | Caller |
| `details::{add_artwork,clear_artwork,add_playlist_artwork}` | Image references and, when added, files in the Library share tree | Yes, track values or named `playlist_edit`; image bytes are not separately retained for Unwind | Window |
| `commands::reload_tags` | Audio-tag metadata/artwork imported into native fields | Yes, recorded track edits; audio is read by the native reload, not by the Ledger | Window |
| `commands::{create_playlist,create_smart_playlist,create_folder,set_smart_rule,rename_playlist,move_playlist,sort_playlist_items,delete_playlist}` | Playlist/folder rows, smart rules and tree order | Yes, named `create_playlist`, `playlist_edit`, `rename_playlist`, `move_playlist`, `sort_playlists`, `delete_playlist` | Window |
| `commands::{add_tracks_to_playlist,remove_tracks_from_playlist,reorder_playlist}` | Membership/order rows | Yes, named history kinds; sliced additions carry `part_of` links and one quick Undo | Window |
| `playlist_entries::reorder_playlist_entries`; service `playlist_edits::{remove,delete,reorder,copy}` | Entry order, membership, playlist deletion/copy | Yes; entry reorder has values, other kinds named. Mutant Follows/copies are coordinated by the existing Ledger hooks. | Window or Caller |
| `commands::{add_to_tag_list,remove_from_tag_list,clear_tag_list,remove_from_history}` | Tag-list/history membership and ordering | Yes, named outside-history or `list_removal` history entries | Window |
| `commands::{record_play,reset_play_count}` | Play history/session rows and play counts | Yes, named `record_play` outside-history entry; reset has track values | Window |
| `commands::relocate_track`; `relocate::{relocate_by_location,auto_relocate}` | Track file links/related native relocation fields | Yes, named `relocate` history entry. The search/preview itself does not write. | Window |
| `commands::remove_from_collection`; `relocate::remove_missing_tracks`; service `playlist_edits::remove_from_collection` | Permanent native track/reference deletion; related Mutants reconciled | Yes, named `permanent` entry; no durable native inverse | Window or Caller |
| `cues::{add_cue,add_loop,move_cue,move_hot_cue,set_cue_colour,set_cue_comment,set_active_loop,delete_cue,convert_memory_cues_to_hot}` via `edit_cues` | Cue/loop rows and revisions | Yes, named `cues` history entry | Window |
| `grid::{guarded,set_tempo}`; Info BPM; grid panel/shared Undo/Redo | Beat-grid analysis files, BPM/revision rows | Yes, `window_grid` with already-held beat values in file intent metadata; history links/grouping retained | Window |
| `grid::grid_lock` | Native analysis-lock column, plus local editor lock cleanup | **Gap:** unnamed `state.write`, no entry or source retained. Its old comment saying nothing in the Library changes is inaccurate. | None recorded; window initiated |
| `analysis::edit_phrase` / `phrase_edit` | Phrase cut/clear in EXT | Yes, `phrase_cut` / `phrase_clear`, cheap phrase values; no Unwind implementation | Window |
| `analysis::analyse_track` / `analyse_and_save_notated`, `save_analysis_files`, `save_key_only`, first-beat cue helper | Native analysis registration, detected-key vocabulary/key, artwork, analysis files, optional first-beat cue | Yes, Analysis Operation; named accepted intermediate steps and returned outcome values, not old audio/artifact snapshots | `tool:wreckordbox-analysis`, one Run per analysis operation |
| Analysis key-tag preference, `key_tag::prepared::write_analysis_key_tag` | DJ-owned audio file's key tag after native analysis | Yes, same Analysis Operation, `{path,before,after}` under `audio_key_tag`; final publication has guarded admission | `tool:wreckordbox-analysis`, same bulk Run |
| `commands::{import_files,import_files_slice}` | New tracks/metadata/lookup/artwork rows and files | Yes, Import Operation; slices share the supplied operation token | Window, or captured Caller at the recorder seam |
| `import_recording::import_collection_recorded`: `commands::{import_xml,import_itunes,import_itunes_selected}` | Imported tracks, cues/metadata, playlists and replacement membership | Yes, Import Operation. Kept replacement history joins its Run, retains Mutant preparation and quick Undo/Redo; no DJ Facts. | Initiator: Window or captured Caller |
| `import_recording::import_folder_recorded`: folder drop/import | Tracks, playlists/folders; Replace existing playlist | Yes, Import Operation; kept `replace_playlist` history joins it | Initiator: Window or captured Caller |
| `device_library::import_device_playlist` and `import_recording::import_device_rows_recorded` | Copied audio, tracks/playlist rows and carried native/analysis metadata | Yes, one Import Operation; names copied/imported outcomes. Failed native admission rolls copies back before recording shutdown. Source Stick is read-only. | Initiator: Window or captured Caller |
| LINK `StateSource::edit`, `player_cue_edit`, `edit_player`, `history_write` | Rating, cue rows/colours, Tag List, hot-cue bank and player history | Yes, player Operation. Ratings use track field values/assertions; other edits are named. No DJ Facts. Partial failures name bounded track barriers. | `tool:link`, one Run per changed message |
| LINK `save_grid_offset` | Analysis grid-offset file (not the row-edit routes above) | Yes, `link_grid_offset`, integer before/after; same-value offset skipped | `tool:link`, existing player Run (separate from per-message Runs) |
| `usb_import::apply`, `usb_import::{ratings,cues}` | Round-trip ratings, cues/analysis/grid metadata and player history | Yes, one Round-trip Operation with bounded slices. Player rating assertions preserve same-value choices when the export baseline proves a change. Partial failure barriers name attempted tracks. Source Stick is read-only. | `tool:round-trip`, one Run per Round-trip |
| `commands::{undo_edit,redo_edit}` / `LibraryCore::step_history` | The inverse/reapplication of each kept native edit and its supported file changes | Yes, same recording point, `undo`/`redo` and `reverses`; original payload direction preserved, shown values reversed for Undo. Audio preparation/cleanup is outside the Library gate. | Window, regardless of original edit's source |
| Service `projection::write` (`projection.apply`) | Chosen supported Fact values in native fields | Yes, recorded track values/ownership marks in one Run. Dry run writes nothing. | Default `tool:projection`, or explicit non-DJ Caller; DJ attribution refused |
| Scripting `cocoa` setters/playlist operations, `my_tags::{update,reorder}`, `collection::remove` | Same native mutations as their delegated window commands | Yes, same recorded/named/permanent path as those commands | Window/DJ; scripting does not supply a service caller source |

## Recovery and boundaries

| Path | Coverage / source |
|---|---|
| `file_journal::recover`, called on owning startup or before a write | Reconciles an interrupted file/row operation. Where a Ledger intent exists, recovery retains that operation's original attribution and an unconfirmed/recorded outcome. It does not create a new per-file recovery Run. Legacy journal work without an intent has no fresh Ledger entry. This is lifecycle/write recovery, not a read endpoint. |
| Ledger spill/recovery and pending native intents | Append/replay keeps the original source, Run and acceptance order. Reads expose pending records and never replay. Recording failure cannot roll back an accepted Library write. |
| Backup restore (`rbl-backup` restore/journal, standalone Restore tool and generated restore scripts) | **Not a new Ledger operation.** Restores the saved Library and Sidecar, including the Ledger snapshot. The external restore initiator is not recorded as a new source/Run. Owning startup can finish/roll back its journal. |
| rekordbox or another external application while wreckordbox is closed | **Not intercepted or recorded.** Later native conflict/ownership checks can preserve changed values but cannot reconstruct the external actor's Run. |
| No hosted Ledger / standalone low-level Writer consumer | **No durable Ledger.** Native guards/history alone do not install one. Production coverage above depends on the service owning the Library and attaching its Keeper. |
| `wrote_outside_history`, history clearing and index reload | Bookkeeping only, not independent native writes. Their surrounding operation owns any accepted mutation. |
| Backups, preferences, played markers, local grid editor settings, logging | Separate output/state, not Library row/audio publication. No Library Ledger entry claimed. |
| Facts, Decisions, Stems, Mutants and tag-schema files | Sidecar/configuration paths with their own provenance, not native Library writes. Mutant Follows accompanying a native edit are coordinated with its Ledger entry. |
| Export/sync/Stick edits, Native stems and rendered Stem/Mutant tracks on a Stick | Stick writes, outside this Library coverage audit. Existing held confirmation/Stick authorization remains mandatory; no change to those paths here. |

## Remaining work

The analysis-lock gap is tracked by **wbx-14.12**: named recording and a
refused-write regression.
Restore/external-writer boundaries above must not be mistaken for automatic
coverage. Recorded named-only history kinds still need durable inverse payloads
before Unwind can reverse them. Audio key tags have values and quick Undo/Redo,
not a crash-atomic transaction across audio and SQLite or an Unwind implementation.
The earlier bulk/player/import review hardening remains listed in the bead and
implementation notes; this audit does not silently mark it complete.

The audit was traced from native write wrappers, file publication helpers,
service Run admission, window command registration and scripting dispatch.
It is a source inventory, not hardware verification or a new runtime scanner.
