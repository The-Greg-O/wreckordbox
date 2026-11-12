//! What an export or a sync writes to a Stick, built from the Library: the
//! window's Export Playlist, Export Track and Sync Manager build it here, and
//! so does the service's Stick load (wbx-70), so a plan and the write it
//! confirms read the Library the same way.
//!
//! Moved unchanged from the window's `commands.rs`; the Library is reached
//! through [`LibrarySource`] (the window's state, or the service's Library).

use rbl_db::DbError;

use crate::{Result as AppResult, StickWriteError};

/// The Library a selection is read from: the Library core's two gates (a
/// selection is read holding both, so no edit or analysis write lands
/// half-way through it) and a read-only connection for what the index does
/// not hold. Nothing is written through it.
pub trait LibrarySource {
    /// The Library core's edit gate (`rbl_library::LibraryCore::edit_gate`).
    fn edit_gate(&self) -> &parking_lot::ReentrantMutex<()>;
    /// The Library core's analysis lock (`rbl_library::LibraryCore::analysis_write`).
    fn analysis_write(&self) -> &parking_lot::Mutex<()>;
    /// Runs `read` on a read-only connection to the Library.
    fn read_db(
        &self,
        read: &mut dyn FnMut(&rbl_db::Library) -> Result<(), DbError>,
    ) -> Result<(), DbError>;
}

/// [`LibrarySource::read_db`] answering a value.
pub fn read_db<T>(
    source: &dyn LibrarySource,
    read: impl FnOnce(&rbl_db::Library) -> Result<T, DbError>,
) -> Result<T, DbError> {
    let mut read = Some(read);
    let mut out = None;
    source.read_db(&mut |db| {
        if let Some(read) = read.take() {
            out = Some(read(db)?);
        }
        Ok(())
    })?;
    out.ok_or_else(|| DbError::WriteRefused("the Library was not read".to_owned()))
}

/// The library playlists a stick holds from this library, in the order its
/// manifest lists them, and whether its record asks for automatic sync:
/// re-exported with what they hold now when something is added.
///
/// Only a manifest written from this library counts: another library's ids
/// name other playlists. A manifest from before rbxport recorded the library
/// counts when rekordbox's sync record names this library, as the export
/// itself accepts it. Folders, playlists since deleted from the library, and
/// the stick's own playlists (made on a player or by rekordbox) are left
/// out: an export with `keep_unlisted` keeps them as the stick has them.
pub fn held_on_stick(
    state: &dyn LibrarySource,
    library: &rbl_index::Library,
    stick: &std::path::Path,
) -> (Vec<String>, bool) {
    let Some(manifest) = rbl_export::Manifest::load(stick) else {
        return (Vec::new(), false);
    };
    let db_id = read_db(state, |db| rbl_db::export_info::db_id(db.connection())).unwrap_or(0);
    let record = rbl_export::sync_record::read(stick);
    let ours = manifest.db_id == db_id
        || (manifest.db_id == 0 && db_id != 0 && record.as_ref().is_some_and(|r| r.db_id == db_id));
    if !ours {
        return (Vec::new(), false);
    }
    let playlists = library.playlists();
    let held = manifest
        .playlists
        .iter()
        .filter(|p| {
            !p.folder
                && !p.device_only
                && p.library_id != 0
                && playlists.index_of(p.library_id).is_some()
        })
        .map(|p| p.library_id.to_string())
        .collect();
    (held, record.is_some_and(|r| r.automatic))
}

/// What Export Track writes to `stick`: what it already holds, with
/// `tracks` added in no playlist.
pub fn track_export_selection(
    state: &dyn LibrarySource,
    library: &rbl_index::Library,
    share: &std::path::Path,
    stick: &std::path::Path,
    tracks: &[String],
) -> AppResult<ExportSelection> {
    if !stick.is_dir() {
        return Err(StickWriteError::NotFound(
            "That device is no longer connected.".to_owned(),
        ));
    }
    let (playlists, automatic) = held_on_stick(state, library, stick);
    let mut loose = rbl_export::Manifest::load(stick)
        .map(|m| m.loose)
        .unwrap_or_default();
    for id in tracks.iter().filter_map(|t| t.parse::<u64>().ok()) {
        if !loose.contains(&id) {
            loose.push(id);
        }
    }
    let mut selection = ExportSelection::from_playlists_and_tracks(
        state, library, share, &playlists, &loose, automatic,
    )?;
    selection.keep_unlisted = true;
    Ok(selection)
}

/// A My Tag row as the export takes it.
pub fn source_my_tag(tag: &rbl_db::export_info::MyTagRow) -> rbl_export::SourceMyTag {
    rbl_export::SourceMyTag {
        id: tag.id.parse().unwrap_or(0),
        seq: u32::try_from(tag.seq.max(0)).unwrap_or(u32::MAX),
        name: tag.name.clone(),
        attribute: u8::try_from(tag.attribute.clamp(0, 255)).unwrap_or(0),
        parent: tag.parent.parse().unwrap_or(0),
    }
}

/// What an export is asked to write: the tracks, the playlists that name
/// them by index, and the library's My Tags. Built once and written to as
/// many sticks as asked.
#[derive(Clone)]
pub struct ExportSelection {
    pub tracks: Vec<rbl_export::SourceTrack>,
    pub playlists: Vec<rbl_export::SourcePlaylist>,
    /// Every My Tag of the library, listed on the stick whole as rekordbox
    /// lists them.
    pub my_tags: Vec<rbl_export::SourceMyTag>,
    /// What the stick's sync record names: the library and its tree.
    pub sync: rbl_export::SyncSource,
    /// Add to what the stick holds (Export Playlist, Export Track) rather
    /// than make it hold exactly this (a SYNC): see
    /// [`rbl_export::ExportOptions::keep_unlisted`].
    pub keep_unlisted: bool,
}

/// What exporting the folder at `folder` writes, in tree order: the
/// playlists (plain and intelligent) under it at any depth, and the folder
/// itself with every folder under it, so empty ones keep their place.
fn folder_contents(playlists: &rbl_index::Playlists, folder: usize) -> (Vec<usize>, Vec<usize>) {
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); playlists.len()];
    for index in 0..playlists.len() {
        if let Some(parent) = playlists
            .parent
            .get(index)
            .copied()
            .filter(|&p| p != rbl_index::NO_ID)
        {
            if let Some(bucket) = children.get_mut(parent as usize) {
                bucket.push(index);
            }
        }
    }
    let (mut leaves, mut folders) = (Vec::new(), Vec::new());
    // Iterative, with a visited set, as the tree builder is: a corrupt
    // parent cycle must not loop forever.
    let mut visited = vec![false; playlists.len()];
    let mut stack = vec![folder];
    while let Some(index) = stack.pop() {
        match visited.get_mut(index) {
            Some(seen) if !*seen => *seen = true,
            _ => continue,
        }
        if playlists.is_folder(index) {
            folders.push(index);
            if let Some(under) = children.get(index) {
                stack.extend(under.iter().rev());
            }
        } else {
            leaves.push(index);
        }
    }
    (leaves, folders)
}

/// Each playlist and folder id's place in the tree read top to bottom: a
/// folder, then what is inside it, each level in `Seq` order (a tie by row).
fn tree_rank(playlists: &rbl_index::Playlists) -> std::collections::HashMap<u64, usize> {
    let mut children: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
    for index in 0..playlists.len() {
        let parent = playlists
            .parent
            .get(index)
            .copied()
            .unwrap_or(rbl_index::NO_ID);
        children.entry(parent).or_default().push(index);
    }
    for siblings in children.values_mut() {
        siblings.sort_by_key(|&i| (playlists.seq.get(i).copied().unwrap_or(0), i));
    }
    let mut rank = std::collections::HashMap::new();
    let mut visited = vec![false; playlists.len()];
    let mut stack: Vec<usize> = children
        .get(&rbl_index::NO_ID)
        .map(|c| c.iter().rev().copied().collect())
        .unwrap_or_default();
    while let Some(index) = stack.pop() {
        match visited.get_mut(index) {
            Some(seen) if !*seen => *seen = true,
            _ => continue,
        }
        rank.insert(playlists.ids.get(index).copied().unwrap_or(0), rank.len());
        if let Some(under) = u32::try_from(index).ok().and_then(|i| children.get(&i)) {
            stack.extend(under.iter().rev());
        }
    }
    rank
}

impl ExportSelection {
    /// The union of these playlists, each track read once however many of
    /// them hold it. Ids are the tree's numeric playlist ids. An intelligent
    /// playlist is exported as what its rule admits now, which is what
    /// rekordbox writes to a stick for one too. A folder stands for every
    /// playlist under it, and goes on the stick as a folder with them inside.
    pub fn from_playlists(
        state: &dyn LibrarySource,
        library: &rbl_index::Library,
        share: &std::path::Path,
        playlist_ids: &[String],
        // Written into the stick's sync record: whether it is to be synced
        // again, on its own, when it is next plugged in.
        automatic: bool,
    ) -> AppResult<Self> {
        Self::from_playlists_and_tracks(state, library, share, playlist_ids, &[], automatic)
    }

    /// [`from_playlists`](Self::from_playlists) with tracks that go on the
    /// stick in no playlist: what Export Track put there, and what a
    /// stick's record says was put there before.
    #[allow(
        clippy::too_many_lines,
        reason = "moved unchanged from the window's commands, which the window's lints did not hold to this"
    )]
    pub fn from_playlists_and_tracks(
        state: &dyn LibrarySource,
        library: &rbl_index::Library,
        share: &std::path::Path,
        playlist_ids: &[String],
        loose: &[u64],
        automatic: bool,
    ) -> AppResult<Self> {
        let _editing = state.edit_gate().lock();
        let _analysis = state.analysis_write().lock();
        // Named first and read after: `source_rows` takes the playlists
        // itself, so the guard is let go before it is asked.
        let mut named: Vec<(u64, String, rbl_index::TrackSource)> =
            Vec::with_capacity(playlist_ids.len());
        // Folders asked for by name, kept on the stick even when nothing
        // under them is a playlist.
        let mut chosen_folders: Vec<u64> = Vec::new();
        {
            let playlists = library.playlists();
            let mut seen = std::collections::HashSet::new();
            for id in playlist_ids {
                let Some(index) = id
                    .parse::<u64>()
                    .ok()
                    .and_then(|numeric| playlists.index_of(numeric))
                else {
                    return Err(StickWriteError::NotFound(
                        "That playlist is not in the library.".to_owned(),
                    ));
                };
                // A folder has no tracks of its own: exported as a playlist
                // it would land on the stick empty.
                let (leaves, folders) = if playlists.is_folder(index) {
                    folder_contents(&playlists, index)
                } else {
                    (vec![index], Vec::new())
                };
                chosen_folders.extend(
                    folders
                        .into_iter()
                        .filter_map(|folder| playlists.ids.get(folder).copied()),
                );
                for index in leaves {
                    if !seen.insert(index) {
                        continue;
                    }
                    let source = if playlists.is_smart(index) {
                        rbl_index::TrackSource::SmartPlaylist(index)
                    } else {
                        rbl_index::TrackSource::Playlist(index)
                    };
                    named.push((
                        playlists.ids.get(index).copied().unwrap_or(0),
                        playlists.name(index).to_owned(),
                        source,
                    ));
                }
            }
        }
        let rows_of: Vec<Vec<u32>> = named
            .iter()
            .map(|(_, _, source)| library.source_rows(source))
            .collect();
        // A loose track the library no longer has is left off without a
        // word: the stick's record outlives the track.
        let loose_rows: Vec<u32> = loose
            .iter()
            .filter_map(|&id| library.row_of_id(id))
            .collect();

        // What the index does not hold: the My Tags, and the other places a
        // cloud-synced file may be. One read for the whole selection.
        let ids: Vec<String> = rows_of
            .iter()
            .flatten()
            .chain(loose_rows.iter())
            .map(|&row| {
                library
                    .ids
                    .get(row as usize)
                    .copied()
                    .unwrap_or(0)
                    .to_string()
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        // When each plain playlist's entries last changed, which a stick
        // rekordbox synced keeps in its record (#471).
        let plain: Vec<u64> = named
            .iter()
            .filter(|(_, _, source)| matches!(source, rbl_index::TrackSource::Playlist(_)))
            .map(|(id, _, _)| *id)
            .collect();
        let (my_tags, extras, (db_id, cloud_db_id), entries_changed) = read_db(state, |db| {
            let conn = db.connection();
            Ok((
                rbl_db::export_info::my_tags(conn)?,
                rbl_db::export_info::track_extras(conn, &ids)?,
                (
                    rbl_db::export_info::db_id(conn)?,
                    rbl_db::export_info::cloud_db_id(conn)?,
                ),
                rbl_db::export_info::playlist_entries_changed(conn, &plain)?,
            ))
        })?;
        // The whole tree goes along; the record keeps the ticked playlists
        // and the folders above them.
        let tree = {
            let playlists = library.playlists();
            (0..playlists.len())
                .map(|index| rbl_export::SyncNode {
                    id: playlists.ids.get(index).copied().unwrap_or(0),
                    parent: match playlists.parent.get(index).copied() {
                        Some(parent) if parent != rbl_index::NO_ID => {
                            playlists.ids.get(parent as usize).copied().unwrap_or(0)
                        }
                        _ => 0,
                    },
                    attribute: playlists.attribute.get(index).copied().unwrap_or(0),
                })
                .collect()
        };
        let sync = rbl_export::SyncSource {
            db_id,
            tree,
            automatic,
            cloud_db_id,
        };
        let my_tags: Vec<rbl_export::SourceMyTag> = my_tags.iter().map(source_my_tag).collect();

        let mut tracks: Vec<rbl_export::SourceTrack> = Vec::new();
        let mut position: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        let mut source_playlists = Vec::with_capacity(named.len());
        for ((id, name, _), rows) in named.into_iter().zip(rows_of) {
            let mut track_indices = Vec::with_capacity(rows.len());
            for row in rows {
                let at = if let Some(at) = position.get(&row) {
                    *at
                } else {
                    let content = library
                        .ids
                        .get(row as usize)
                        .copied()
                        .unwrap_or(0)
                        .to_string();
                    let extra = extras.get(&content).cloned().unwrap_or_default();
                    tracks.push(source_track(library, share, row, &extra));
                    let at = tracks.len() - 1;
                    position.insert(row, at);
                    at
                };
                track_indices.push(at);
            }
            let entries_changed_ms = entries_changed.get(&id).copied();
            source_playlists.push(rbl_export::SourcePlaylist {
                device_id: 0,
                device_only: false,
                parent_id: 0,
                folder: false,
                id,
                name,
                track_indices,
                entries_changed_ms,
            });
        }
        for row in loose_rows {
            if let std::collections::hash_map::Entry::Vacant(entry) = position.entry(row) {
                let content = library
                    .ids
                    .get(row as usize)
                    .copied()
                    .unwrap_or(0)
                    .to_string();
                let extra = extras.get(&content).cloned().unwrap_or_default();
                tracks.push(source_track(library, share, row, &extra));
                entry.insert(tracks.len() - 1);
            }
        }
        // Include ancestors as actual folder rows in both USB databases.
        let tree_view = library.playlists();
        let mut ancestors = std::collections::HashSet::new();
        for p in &mut source_playlists {
            p.parent_id = sync
                .tree
                .iter()
                .find(|n| n.id == p.id)
                .map_or(0, |n| n.parent);
            let mut parent = p.parent_id;
            while parent != 0 && ancestors.insert(parent) {
                parent = sync
                    .tree
                    .iter()
                    .find(|n| n.id == parent)
                    .map_or(0, |n| n.parent);
            }
        }
        for folder in chosen_folders {
            let mut next = folder;
            while next != 0 && ancestors.insert(next) {
                next = sync
                    .tree
                    .iter()
                    .find(|n| n.id == next)
                    .map_or(0, |n| n.parent);
            }
        }
        let mut folders = Vec::new();
        for id in ancestors {
            let Some(index) = tree_view.index_of(id) else {
                return Err(StickWriteError::internal("Missing playlist ancestor"));
            };
            folders.push(rbl_export::SourcePlaylist {
                device_id: 0,
                device_only: false,
                id,
                name: tree_view.name(index).to_owned(),
                parent_id: sync
                    .tree
                    .iter()
                    .find(|n| n.id == id)
                    .map_or(0, |n| n.parent),
                folder: true,
                track_indices: Vec::new(),
                entries_changed_ms: None,
            });
        }
        folders.append(&mut source_playlists);
        source_playlists = folders;
        // The library tree's own order, folders and playlists as the tree
        // lists them: the exporter numbers siblings in the order given, and
        // the ancestors were gathered in id order (#315).
        let rank = tree_rank(&tree_view);
        source_playlists.sort_by_key(|p| rank.get(&p.id).copied().unwrap_or(usize::MAX));
        Ok(Self {
            tracks,
            playlists: source_playlists,
            my_tags,
            sync,
            keep_unlisted: false,
        })
    }

    /// This selection with the loose tracks a stick's record names added,
    /// so a sync keeps what Export Track put there unless cleanup is enabled.
    /// Cleanup uses the playlist selection alone; the exporter removes only
    /// stale manifest-owned files after publishing the new databases. The selection itself
    /// when the record names none it does not already hold.
    ///
    /// Only reads the Stick: nothing in this crate writes to one. A write
    /// that settles an unfinished export first does so itself, under its
    /// own guard (the window's `commands::for_stick`).
    pub fn for_stick_inspected<'a>(
        &'a self,
        state: &dyn LibrarySource,
        library: &rbl_index::Library,
        share: &std::path::Path,
        destination: &std::path::Path,
        delete_unlisted_music: bool,
    ) -> AppResult<std::borrow::Cow<'a, Self>> {
        if delete_unlisted_music {
            return Ok(std::borrow::Cow::Borrowed(self));
        }
        let recorded = rbl_export::Manifest::load(destination)
            .filter(|m| m.db_id == self.sync.db_id)
            .map(|m| m.loose)
            .unwrap_or_default();
        let held: std::collections::HashSet<u64> = self.tracks.iter().map(|t| t.id).collect();
        let missing: Vec<u64> = recorded
            .into_iter()
            .filter(|id| !held.contains(id))
            .collect();
        if missing.is_empty() {
            return Ok(std::borrow::Cow::Borrowed(self));
        }
        let playlist_ids: Vec<String> = self
            .playlists
            .iter()
            .filter(|p| !p.folder)
            .map(|p| p.id.to_string())
            .collect();
        let loose: Vec<u64> = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                !self
                    .playlists
                    .iter()
                    .any(|p| p.track_indices.contains(index))
            })
            .map(|(_, t)| t.id)
            .chain(missing)
            .collect();
        let mut kept = Self::from_playlists_and_tracks(
            state,
            library,
            share,
            &playlist_ids,
            &loose,
            self.sync.automatic,
        )?;
        kept.keep_unlisted = self.keep_unlisted;
        Ok(std::borrow::Cow::Owned(kept))
    }
}

/// One library row as the export wants it, analysis read from the share
/// tree, and what the index does not hold from `extra`.
fn source_track(
    library: &rbl_index::Library,
    share: &std::path::Path,
    row: u32,
    extra: &rbl_db::export_info::TrackExtras,
) -> rbl_export::SourceTrack {
    let i = row as usize;
    // What analysis wrote, from the row as it is now: an analysis run
    // re-reads the index only once it is over, and a sync made during one
    // must not take the tracks it has analysed as unanalysed (#309).
    let analysed = extra.analysis.as_ref();
    let analysis_path = analysed.map_or_else(
        || library.analysis_path.get(i),
        |a| a.analysis_path.as_str(),
    );
    let artwork_path =
        analysed.map_or_else(|| library.artwork_path.get(i), |a| a.artwork_path.as_str());
    // The library's own image, share-relative like the analysis.
    let artwork = Some(artwork_path)
        .filter(|p| !p.is_empty())
        .map(|p| share.join(p.trim_start_matches(['/', '\\'])));
    rbl_export::SourceTrack {
        cues: Some(extra.cues.clone()),
        metadata: extra.metadata.clone(),
        device: None,
        // The content id is how a second export to the same stick
        // recognises a track it has already written.
        id: library.ids.get(i).copied().unwrap_or(0),
        source_path: source_audio(library.folder_path.get(i), &extra.alternate_paths),
        artwork,
        my_tags: extra
            .my_tags
            .iter()
            .filter_map(|t| t.parse().ok())
            .collect(),
        title: library.title.get(i).to_owned(),
        artist: library.artist_name(row).to_owned(),
        album: library.album_name(row).to_owned(),
        genre: library.genre_name(row).to_owned(),
        label: library.label_name(row).to_owned(),
        key: analysed.map_or_else(|| library.key_name(row).to_owned(), |a| a.key.clone()),
        comment: library.comment.get(i).to_owned(),
        date_added: library.date_added.get(i).to_owned(),
        release_date: library.release_date.get(i).to_owned(),
        bpm_x100: analysed.map_or_else(
            || library.bpm_x100.get(i).copied().unwrap_or(0),
            |a| a.bpm_x100,
        ),
        duration_sec: u16::try_from(analysed.map_or_else(
            || library.length_sec.get(i).copied().unwrap_or(0),
            |a| a.length_sec,
        ))
        .unwrap_or(u16::MAX),
        rating: library.rating.get(i).copied().unwrap_or(0),
        color_id: library.color.get(i).copied().unwrap_or(0),
        bitrate: library.bitrate.get(i).copied().unwrap_or(0),
        sample_rate: library.sample_rate.get(i).copied().unwrap_or(0),
        file_size: library.file_size.get(i).copied().unwrap_or(0),
        year: library.year.get(i).copied().unwrap_or(0),
        analysis: Vec::new(),
        analysis_source: (!analysis_path.is_empty())
            .then(|| share.join(analysis_path.trim_start_matches(['/', '\\']))),
        source_unavailable: false,
        fallback: None,
        prepared_audio: None,
        preparation_error: None,
        retained_reason: None,
        cache_bypass: false,
    }
}

/// Where a track's audio is read from.
///
/// `FolderPath` when the file is there. A cloud-synced track's `FolderPath`
/// can name a copy that is not (the Dropbox one, on a machine where the
/// folder is not synced down), so the other paths the row names —
/// `rb_LocalFolderPath`, then `OrgFolderPath` — are tried in turn, and
/// the first that exists is the source. None existing leaves `FolderPath`,
/// so the export reports the track as skipped under the name the row gives.
fn source_audio(folder_path: &str, alternates: &[String]) -> std::path::PathBuf {
    let first = std::path::PathBuf::from(folder_path);
    if first.is_file() {
        return first;
    }
    alternates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
        .unwrap_or(first)
}
