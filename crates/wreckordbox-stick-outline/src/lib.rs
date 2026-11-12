//! A mounted Stick's playlist trees, for showing them: read only.
//!
//! The shell's tree lists every mounted Stick's playlists without anyone
//! opening the Stick, so this read must never write to it. The device
//! library's own reader (`rbl_export::device_library::read`) first finishes
//! or undoes an interrupted export (`rbl_export::recover`), which renames,
//! replaces and deletes files on the Stick; that belongs to an explicit
//! operation only. Here an unfinished publication is reported as `pending`
//! and nothing is listed, exactly as device discovery
//! (`rbl_devices::inspect`) does.
//!
//! Nothing here creates, opens for writing or locks anything new: the
//! shared lease is taken only on a write lock that already exists, the
//! `export.pdb` is read into memory and the `exportLibrary.db` is opened
//! read-only. Only the playlist tree and each playlist's track count are
//! read, not the tracks themselves.
//!
//! Added beside the protected crates rather than inside them (ADR 0003): the
//! node ordering repeats `device_library`'s so a row's id and position are the
//! ones the existing Stick view opens.

mod library;
#[cfg(feature = "testing")]
pub mod testing;

pub use library::{library, read, settle, unfinished_write, ReadError, StickRead};

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rbl_export::device_library::{self as device, Format};
use serde::Serialize;

/// What discovery found on a Stick.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outline {
    /// An earlier write to the Stick did not finish. Nothing is listed until
    /// an explicit operation settles it.
    pub pending: bool,
    /// Device Library first, then `OneLibrary`, as rekordbox lists them.
    pub libraries: Vec<LibraryOutline>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryOutline {
    pub format: &'static str,
    /// Depth first, siblings in their order.
    pub nodes: Vec<NodeOutline>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeOutline {
    /// Sent as text, as the device library's own tree sends ids.
    #[serde(serialize_with = "as_text")]
    pub id: u32,
    /// 0 for the top level.
    #[serde(rename = "parentId", serialize_with = "as_text")]
    pub parent: u32,
    pub name: String,
    pub folder: bool,
    pub depth: u32,
    /// Tracks in a playlist; children of a folder.
    pub count: u32,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's signature
fn as_text<S: serde::Serializer>(id: &u32, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(id)
}

const fn format_name(format: Format) -> &'static str {
    match format {
        Format::DeviceLibrary => "deviceLibrary",
        Format::OneLibrary => "oneLibrary",
    }
}

/// A playlist or folder as read, before ordering.
struct Raw {
    id: u32,
    parent: u32,
    name: String,
    folder: bool,
    sequence: u32,
    count: u32,
}

/// Reads the playlist trees of the Stick mounted at `mount`, writing nothing.
///
/// A volume with no export, or a library that cannot be read, lists nothing
/// for it; so does a Stick that cannot be read at all (see [`read_outline`]
/// for why).
#[must_use]
pub fn outline(mount: &Path) -> Outline {
    read_outline(mount).unwrap_or_else(|error| {
        tracing::warn!(stick = %mount.display(), %error, "cannot outline a Stick");
        Outline::default()
    })
}

/// [`outline`], saying why a Stick could not be read at all: its lock could
/// not be leased, or its path named another folder by the end of the read.
/// An unfinished write is an outline with `pending`, not an error.
///
/// # Errors
///
/// [`ReadError::Io`] or [`ReadError::Moved`].
pub fn read_outline(mount: &Path) -> Result<Outline, ReadError> {
    let pending = Outline {
        pending: true,
        libraries: Vec::new(),
    };
    let stick = match StickRead::begin(mount) {
        Ok(stick) => stick,
        Err(ReadError::UnfinishedWrite) => return Ok(pending),
        Err(error) => return Err(error),
    };
    let found = outline_of(stick.root());
    match stick.finish() {
        Ok(()) => Ok(found),
        Err(ReadError::UnfinishedWrite) => Ok(pending),
        Err(error) => Err(error),
    }
}

fn outline_of(mount: &Path) -> Outline {
    let Ok(root) = rbl_export::export_root_name(mount) else {
        return Outline::default();
    };
    let dir = mount.join(root).join("rekordbox");
    let libraries = device::formats(mount)
        .into_iter()
        .filter_map(|format| {
            let path = dir.join(format.file_name());
            let raw = match format {
                Format::DeviceLibrary => read_pdb(&path),
                Format::OneLibrary => read_one(&path),
            };
            match raw {
                Ok(raw) => Some(LibraryOutline { format: format_name(format), nodes: tree_order(raw) }),
                Err(error) => {
                    tracing::warn!(stick = %mount.display(), library = format.file_name(), %error, "cannot outline a Stick library");
                    None
                }
            }
        })
        .collect();
    Outline {
        pending: false,
        libraries,
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn read_pdb(path: &Path) -> Result<Vec<Raw>, String> {
    use rbl_pdb::PageType;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let pdb = rbl_pdb::Pdb::parse(&bytes).map_err(|e| e.to_string())?;
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for entry in pdb
        .table(PageType::PlaylistEntries)
        .map(|t| pdb.playlist_entries(t))
        .unwrap_or_default()
    {
        *counts.entry(entry.playlist_id).or_default() += 1;
    }
    Ok(pdb
        .table(PageType::PlaylistTree)
        .map(|t| pdb.playlist_nodes(t))
        .unwrap_or_default()
        .into_iter()
        .map(|n| Raw {
            count: if n.is_folder {
                0
            } else {
                count(counts.get(&n.id).copied().unwrap_or(0))
            },
            id: n.id,
            parent: n.parent_id,
            name: n.name,
            folder: n.is_folder,
            sequence: n.sort_order,
        })
        .collect())
}

fn read_one(path: &Path) -> Result<Vec<Raw>, String> {
    let db = rbl_onelibrary::ExportLibrary::open_read_only(path).map_err(|e| e.to_string())?;
    let conn = db.connection();
    let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
    let mut statement = conn
        .prepare("SELECT playlist_id, COUNT(*) FROM playlist_content GROUP BY playlist_id")
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (playlist, n) = row.map_err(|e| e.to_string())?;
        counts.insert(playlist, usize::try_from(n).unwrap_or(0));
    }
    let mut statement = conn
        .prepare(
            "SELECT playlist_id, COALESCE(playlist_id_parent, 0), COALESCE(name, ''),
                    COALESCE(attribute, 0), COALESCE(sequenceNo, 0)
             FROM playlist ORDER BY playlist_id",
        )
        .map_err(|e| e.to_string())?;
    let id = |v: i64| u32::try_from(v).unwrap_or(0);
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (playlist, parent, name, attribute, sequence) = row.map_err(|e| e.to_string())?;
        let folder = attribute == rbl_onelibrary::playlists::FOLDER;
        out.push(Raw {
            id: id(playlist),
            parent: id(parent),
            name,
            folder,
            sequence: id(sequence),
            count: if folder {
                0
            } else {
                count(counts.get(&playlist).copied().unwrap_or(0))
            },
        });
    }
    Ok(out)
}

/// Tree order, as `device_library` lays a library out: depth first from the
/// top level, siblings by sequence then id. A node under a missing parent or
/// a parent that is not a folder is listed at the top level; nodes caught in
/// a cycle of folders are listed last rather than hidden. Iterative, so any
/// depth is safe.
fn tree_order(mut nodes: Vec<Raw>) -> Vec<NodeOutline> {
    let folders: BTreeSet<u32> = nodes.iter().filter(|n| n.folder).map(|n| n.id).collect();
    for node in &mut nodes {
        if node.parent != 0 && (!folders.contains(&node.parent) || node.parent == node.id) {
            node.parent = 0;
        }
    }
    let mut children: BTreeMap<u32, Vec<Raw>> = BTreeMap::new();
    for node in nodes {
        children.entry(node.parent).or_default().push(node);
    }
    for siblings in children.values_mut() {
        siblings.sort_by_key(|n| (n.sequence, n.id));
    }
    let mut out = Vec::new();
    let mut stack: Vec<(Raw, u32)> = children
        .remove(&0)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .map(|n| (n, 0))
        .collect();
    while let Some((mut node, depth)) = stack.pop() {
        if let Some(under) = children.remove(&node.id) {
            if node.folder {
                node.count = count(under.len());
            }
            stack.extend(under.into_iter().rev().map(|n| (n, depth + 1)));
        }
        out.push(outlined(node, depth));
    }
    out.extend(children.into_values().flatten().map(|n| outlined(n, 0)));
    out
}

fn outlined(node: Raw, depth: u32) -> NodeOutline {
    NodeOutline {
        id: node.id,
        parent: node.parent,
        name: node.name,
        folder: node.folder,
        depth,
        count: node.count,
    }
}
