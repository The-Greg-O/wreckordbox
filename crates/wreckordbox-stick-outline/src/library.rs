//! A Stick library read in full (its tracks too) by a reader that cannot
//! settle or write anything, and the one explicit action that does settle an
//! unfinished write.
//!
//! `rbl_export::device_library::read` first runs `rbl_export::recover`, which
//! finishes or rolls back an interrupted publication: it renames, replaces
//! and deletes files on the Stick. Nothing here calls it, or anything that
//! can reach it. The databases are parsed here directly (`export.pdb` read
//! into memory, `exportLibrary.db` opened read-only), the same way
//! `device_library` parses them, into its own `Library` type so every caller
//! keeps working on the same data. Added beside the protected crates (ADR
//! 0003).
//!
//! A read is a [`StickRead`]: it resolves the Stick's path once and works on
//! that resolved root, notes the root's device and inode, takes the shared
//! lease on our writers' lock when the lock exists (never creating it; a
//! lease that cannot be taken is an error), and refuses a Stick with a
//! publication journal. [`StickRead::finish`] then looks again: a journal
//! that appeared while reading, or a path that now names another folder,
//! makes the read's answer unusable. The reader itself never writes, so
//! neither can change the Stick; they only stop a half-published or wrong
//! answer from being shown.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use rbl_export::device_library::{Format, Library, Node, Track};

/// Why a Stick could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// An earlier write to the Stick did not finish (or one began while it
    /// was read). Nothing is read until the DJ settles it ([`settle`]) or a
    /// write they asked for does.
    #[error("A write to this Stick did not finish.")]
    UnfinishedWrite,
    /// The Stick's path named another folder by the end of the read.
    #[error("The Stick changed while it was being read.")]
    Moved,
    /// The library is missing or cannot be parsed.
    #[error("Cannot read the device library: {0}")]
    Unreadable(String),
    /// The Stick, its lock or a file could not be opened.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

const JOURNAL: &str = ".rbxport-publication";

/// Whether an earlier write to the Stick at `mount` did not finish. Looks
/// only for the journal.
#[must_use]
pub fn unfinished_write(mount: &Path) -> bool {
    mount.join(JOURNAL).exists()
}

/// Device and inode: what a path names, whichever way it is spelled.
fn identity(path: &Path) -> std::io::Result<(u64, u64)> {
    let meta = std::fs::metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Ok((0, 0))
    }
}

/// One read of a Stick: see the module documentation.
#[derive(Debug)]
pub struct StickRead {
    mount: PathBuf,
    root: PathBuf,
    identity: (u64, u64),
    _lease: Option<File>,
}

impl StickRead {
    /// Starts a read of the Stick mounted at `mount`.
    ///
    /// # Errors
    ///
    /// [`ReadError::UnfinishedWrite`] for a Stick with a journal; an I/O
    /// error when the Stick or an existing lock cannot be opened or leased.
    pub fn begin(mount: &Path) -> Result<Self, ReadError> {
        let root = std::fs::canonicalize(mount)?;
        let identity = identity(&root)?;
        // Waits out a writer of ours; never creates the lock.
        let lease = rbl_core::durable::read_lease(&root)?;
        if unfinished_write(&root) {
            return Err(ReadError::UnfinishedWrite);
        }
        Ok(Self { mount: mount.to_path_buf(), root, identity, _lease: lease })
    }

    /// The Stick's root as resolved when the read began. Read files under
    /// this, not under the path the caller was given.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Parses one library. Opens files for reading only.
    ///
    /// # Errors
    ///
    /// Why the library could not be read.
    pub fn library(&self, format: Format) -> Result<Library, ReadError> {
        let name = rbl_export::export_root_name(&self.root).map_err(|e| ReadError::Unreadable(e.to_string()))?;
        let path = self.root.join(name).join("rekordbox").join(format.file_name());
        if !path.is_file() {
            return Err(ReadError::Unreadable(format!("This device has no {}.", format.file_name())));
        }
        let (nodes, mut tracks) = match format {
            Format::DeviceLibrary => read_pdb(&std::fs::read(&path)?)?,
            Format::OneLibrary => read_one(&path)?,
        };
        tracks.sort_by_key(|t| t.id);
        Ok(Library { format, nodes: tree_order(nodes), tracks })
    }

    /// Ends the read: whether what was read can be shown.
    ///
    /// # Errors
    ///
    /// [`ReadError::UnfinishedWrite`] when a journal appeared meanwhile;
    /// [`ReadError::Moved`] when the Stick's path names another folder now.
    pub fn finish(self) -> Result<(), ReadError> {
        if unfinished_write(&self.root) {
            return Err(ReadError::UnfinishedWrite);
        }
        if identity(&self.root)? != self.identity || identity(&self.mount).ok() != Some(self.identity) {
            return Err(ReadError::Moved);
        }
        Ok(())
    }
}

/// Runs `read` on the Stick mounted at `mount` as one [`StickRead`].
///
/// # Errors
///
/// The read's own error, or why the Stick could not be read (see
/// [`StickRead::begin`] and [`StickRead::finish`]).
pub fn read<T>(mount: &Path, read: impl FnOnce(&StickRead) -> Result<T, ReadError>) -> Result<T, ReadError> {
    let stick = StickRead::begin(mount)?;
    let value = read(&stick);
    // An unfinished write or a moved Stick outranks whatever the read found.
    stick.finish()?;
    value
}

/// Reads one library of the Stick mounted at `mount`, tracks included,
/// writing nothing to the Stick.
///
/// # Errors
///
/// See [`read`].
pub fn library(mount: &Path, format: Format) -> Result<Library, ReadError> {
    read(mount, |stick| stick.library(format))
}

/// Settles an unfinished write on the Stick mounted at `mount`: finishes the
/// interrupted export when it can, otherwise rolls the Stick back to the
/// files it held before, keeping whatever another program wrote since. This
/// writes to the Stick: call it only for an action the DJ chose.
///
/// # Errors
///
/// Why recovery failed; the Stick is left for another try.
pub fn settle(mount: &Path) -> std::io::Result<()> {
    rbl_export::recover(mount)
}

fn unreadable(e: impl std::fmt::Display) -> ReadError {
    ReadError::Unreadable(e.to_string())
}

// The parsers below follow `rbl_export::device_library`'s, field for field,
// so a library read here equals one read there (the tests compare them).

fn read_pdb(bytes: &[u8]) -> Result<(Vec<Node>, Vec<Track>), ReadError> {
    use rbl_pdb::PageType;
    let pdb = rbl_pdb::Pdb::parse(bytes).map_err(unreadable)?;
    let entries = pdb.table(PageType::PlaylistEntries).map(|t| pdb.playlist_entries(t)).unwrap_or_default();
    let mut members: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
    for entry in entries {
        members.entry(entry.playlist_id).or_default().push((entry.entry_index, entry.track_id));
    }
    let nodes = pdb
        .table(PageType::PlaylistTree)
        .map(|t| pdb.playlist_nodes(t))
        .unwrap_or_default()
        .into_iter()
        .map(|n| {
            let mut tracks = members.remove(&n.id).unwrap_or_default();
            tracks.sort_by_key(|&(index, _)| index);
            Node {
                id: n.id,
                parent: n.parent_id,
                name: n.name,
                folder: n.is_folder,
                sequence: n.sort_order,
                tracks: if n.is_folder { Vec::new() } else { tracks.into_iter().map(|(_, track)| track).collect() },
            }
        })
        .collect();
    let names = |kind| -> BTreeMap<u32, String> {
        pdb.table(kind).map(|t| pdb.named_rows(t).into_iter().map(|n| (n.id, n.name)).collect()).unwrap_or_default()
    };
    let (artists, albums, genres, labels, keys, artwork) = (
        names(PageType::Artists),
        names(PageType::Albums),
        names(PageType::Genres),
        names(PageType::Labels),
        names(PageType::Keys),
        names(PageType::Artwork),
    );
    let name = |map: &BTreeMap<u32, String>, id: u32| map.get(&id).cloned().unwrap_or_default();
    let tracks = pdb
        .table(PageType::Tracks)
        .map(|t| pdb.track_rows(t))
        .unwrap_or_default()
        .into_iter()
        .map(|t| Track {
            id: t.id,
            artist: name(&artists, t.artist_id),
            album: name(&albums, t.album_id),
            genre: name(&genres, t.genre_id),
            label: name(&labels, t.label_id),
            key: name(&keys, t.key_id),
            title: t.title,
            comment: t.comment,
            date_added: t.date_added,
            bpm_x100: t.tempo_x100,
            duration_sec: u32::from(t.duration_sec),
            rating: t.rating.min(5),
            color: t.color_id,
            path: t.file_path,
            analysis_path: t.analyze_path,
            artwork_path: name(&artwork, t.artwork_id),
            master_db_id: u64::from(t.master_db_id),
            master_content_id: u64::from(t.master_content_id),
        })
        .collect();
    Ok((nodes, tracks))
}

/// `exportLibrary.db`'s rating: rekordbox's 0-255 scale or plain stars.
fn onelibrary_stars(raw: i64) -> u8 {
    let stars = if raw > 5 { raw / 51 } else { raw };
    u8::try_from(stars.clamp(0, 5)).unwrap_or(0)
}

fn read_one(path: &Path) -> Result<(Vec<Node>, Vec<Track>), ReadError> {
    let db = rbl_onelibrary::ExportLibrary::open_read_only(path).map_err(unreadable)?;
    let (rows, mut contents) = rbl_onelibrary::playlists::read(db.connection()).map_err(unreadable)?;
    let id = |v: i64| u32::try_from(v).unwrap_or(0);
    let nodes = rows
        .into_iter()
        .map(|row| {
            let folder = row.attribute == rbl_onelibrary::playlists::FOLDER;
            let tracks = contents.remove(&row.id).unwrap_or_default();
            Node {
                id: id(row.id),
                parent: id(row.parent),
                name: row.name,
                folder,
                sequence: id(row.sequence),
                tracks: if folder { Vec::new() } else { tracks.into_iter().map(id).collect() },
            }
        })
        .collect();
    let mut statement = db
        .connection()
        .prepare(
            "SELECT c.content_id, COALESCE(c.title,''), COALESCE(a.name,''), COALESCE(al.name,''),
                    COALESCE(g.name,''), COALESCE(l.name,''), COALESCE(k.name,''), COALESCE(c.djComment,''),
                    COALESCE(c.dateAdded,''), COALESCE(c.bpmx100,0), COALESCE(c.length,0), COALESCE(c.rating,0),
                    COALESCE(c.color_id,0), COALESCE(c.path,''), COALESCE(c.analysisDataFilePath,''), COALESCE(i.path,''),
                    COALESCE(c.masterDbId,0), COALESCE(c.masterContentId,0)
             FROM content c
             LEFT JOIN artist a ON a.artist_id = c.artist_id_artist
             LEFT JOIN album al ON al.album_id = c.album_id
             LEFT JOIN genre g ON g.genre_id = c.genre_id
             LEFT JOIN label l ON l.label_id = c.label_id
             LEFT JOIN key k ON k.key_id = c.key_id
             LEFT JOIN image i ON i.image_id = c.image_id",
        )
        .map_err(unreadable)?;
    let tracks = statement
        .query_map([], |r| {
            Ok(Track {
                id: r.get(0)?,
                title: r.get(1)?,
                artist: r.get(2)?,
                album: r.get(3)?,
                genre: r.get(4)?,
                label: r.get(5)?,
                key: r.get(6)?,
                comment: r.get(7)?,
                date_added: r.get(8)?,
                bpm_x100: r.get(9)?,
                duration_sec: r.get(10)?,
                rating: onelibrary_stars(r.get(11)?),
                color: r.get(12)?,
                path: r.get(13)?,
                analysis_path: r.get(14)?,
                artwork_path: r.get(15)?,
                master_db_id: u64::try_from(r.get::<_, i64>(16)?).unwrap_or(0),
                master_content_id: u64::try_from(r.get::<_, i64>(17)?).unwrap_or(0),
            })
        })
        .map_err(unreadable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(unreadable)?;
    Ok((nodes, tracks))
}

/// Tree order, as `device_library` lays a library out: depth first from the
/// top level, siblings by sequence then id; a node under a missing or
/// non-folder parent at the top level; nodes caught in a cycle listed last.
fn tree_order(mut nodes: Vec<Node>) -> Vec<Node> {
    let folders: BTreeSet<u32> = nodes.iter().filter(|n| n.folder).map(|n| n.id).collect();
    for node in &mut nodes {
        if node.parent != 0 && (!folders.contains(&node.parent) || node.parent == node.id) {
            node.parent = 0;
        }
    }
    let mut children: BTreeMap<u32, Vec<Node>> = BTreeMap::new();
    for node in nodes {
        children.entry(node.parent).or_default().push(node);
    }
    for siblings in children.values_mut() {
        siblings.sort_by_key(|n| (n.sequence, n.id));
    }
    let mut out = Vec::new();
    let mut stack: Vec<Node> = children.remove(&0).unwrap_or_default().into_iter().rev().collect();
    while let Some(node) = stack.pop() {
        if let Some(under) = children.remove(&node.id) {
            stack.extend(under.into_iter().rev());
        }
        out.push(node);
    }
    out.extend(children.into_values().flatten());
    out
}
