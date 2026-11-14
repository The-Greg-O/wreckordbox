//! A Stick's own MyTags (wbx-45.12): its categories and tags, and which of
//! its tracks carry which, read by a [`StickRead`] so nothing can recover or
//! write the Stick.
//!
//! The protected reader's `Library` carries no MyTags (ADR 0003 keeps it as
//! it is), so they are read here, beside it:
//!
//! - **Device Library:** `exportExt.pdb`, read into memory. Type 3
//!   (`DJDBEXMYTAG`) holds one row per category or tag: parent at `0x0c`,
//!   `Seq − 1` at `0x10`, id at `0x14`, 1 at `0x1b` for a category, and the
//!   name at the one-byte offset at `0x1d`. Type 4 (`DJDBEXSONGMYTAG`) holds
//!   one row per tag on a track: the track's `export.pdb` id at `0x04`, the
//!   tag's id at `0x08`. The layout is `rbl_export::ext_pdb`'s, which a
//!   rekordbox 7 file reproduces byte for byte (its module documentation).
//!   A Stick with no `exportExt.pdb` has no MyTags; one that cannot be
//!   parsed is an error, never an empty answer.
//! - **`OneLibrary`:** `exportLibrary.db`'s `myTag` and `myTag_content`,
//!   opened as [`crate::one_library::OneLibraryRead`] opens it (immutable,
//!   or a copy off the Stick). A database without those tables has none.
//!
//! Categories are listed by `Seq` then id, and each category's tags the
//! same way, as rekordbox lists them. A tag whose parent is not a category
//! is left out: no category offers it.

use std::collections::BTreeMap;
use std::path::Path;

use rbl_export::device_library::Format;

use crate::{ReadError, StickRead};

/// One tag: its id in the Stick's library and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickTag {
    pub id: u32,
    pub name: String,
}

/// One category and its tags, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickTagCategory {
    pub id: u32,
    pub name: String,
    pub tags: Vec<StickTag>,
}

/// A Stick library's MyTags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StickMyTags {
    /// In rekordbox's order.
    pub categories: Vec<StickTagCategory>,
    /// Each track's tag ids (by its id in this library), each once, in the
    /// categories' order.
    pub tracks: BTreeMap<u32, Vec<u32>>,
}

/// A row of either table: `(id, seq, name, is_category, parent)`.
type TagRow = (u32, u32, String, bool, u32);

impl StickRead {
    /// Reads one library's MyTags. Opens files for reading only.
    ///
    /// # Errors
    ///
    /// Why the tag file or tables could not be read.
    pub fn my_tags(&self, format: Format) -> Result<StickMyTags, ReadError> {
        let name = rbl_export::export_root_name(self.root())
            .map_err(|e| ReadError::Unreadable(e.to_string()))?;
        let dir = self.root().join(name).join("rekordbox");
        let (rows, pairs) = match format {
            Format::DeviceLibrary => {
                let path = dir.join("exportExt.pdb");
                if !path.is_file() {
                    return Ok(StickMyTags::default());
                }
                read_ext(&std::fs::read(&path)?)
                    .map_err(|e| ReadError::Unreadable(format!("exportExt.pdb: {e}")))?
            }
            Format::OneLibrary => read_one(&dir.join(format.file_name()))
                .map_err(|e| ReadError::Unreadable(format!("exportLibrary.db MyTags: {e}")))?,
        };
        Ok(assemble(rows, pairs))
    }
}

fn read_ext(bytes: &[u8]) -> Result<(Vec<TagRow>, Vec<(u32, u32)>), String> {
    let pdb = rbl_pdb::Pdb::parse(bytes).map_err(|e| e.to_string())?;
    // The ext file's types 3 and 4 share numbers with export.pdb's albums
    // and labels, which is how `rbl_pdb` names them.
    let rows = pdb
        .table(rbl_pdb::PageType::Albums)
        .map(|t| pdb.rows(t))
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let name_at = usize::from(pdb.u1_at(r, 0x1d));
            let name = if name_at == 0 {
                String::new()
            } else {
                pdb.string_at(r.offset + name_at)
            };
            (
                pdb.u4_at(r, 0x14),
                pdb.u4_at(r, 0x10).saturating_add(1),
                name,
                pdb.u1_at(r, 0x1b) == 1,
                pdb.u4_at(r, 0x0c),
            )
        })
        .collect();
    let pairs = pdb
        .table(rbl_pdb::PageType::Labels)
        .map(|t| pdb.rows(t))
        .unwrap_or_default()
        .into_iter()
        .map(|r| (pdb.u4_at(r, 0x04), pdb.u4_at(r, 0x08)))
        .collect();
    Ok((rows, pairs))
}

fn read_one(path: &Path) -> Result<(Vec<TagRow>, Vec<(u32, u32)>), String> {
    let db = crate::one_library::OneLibraryRead::open(path)?;
    let conn = db.connection();
    let has = |table: &str| -> Result<bool, String> {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(|e| e.to_string())
    };
    if !has("myTag")? {
        return Ok((Vec::new(), Vec::new()));
    }
    let id = |v: i64| u32::try_from(v).unwrap_or(0);
    let mut statement = conn
        .prepare(
            "SELECT myTag_id, COALESCE(sequenceNo,0), COALESCE(name,''), COALESCE(attribute,0),
                    COALESCE(myTag_id_parent,0) FROM myTag",
        )
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                id(r.get(0)?),
                id(r.get(1)?),
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)? == 1,
                id(r.get(4)?),
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let pairs = if has("myTag_content")? {
        let mut statement = conn
            .prepare("SELECT content_id, myTag_id FROM myTag_content")
            .map_err(|e| e.to_string())?;
        let pairs = statement
            .query_map([], |r| Ok((id(r.get(0)?), id(r.get(1)?))))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        pairs
    } else {
        Vec::new()
    };
    Ok((rows, pairs))
}

/// Orders the categories and tags and keeps the memberships of tags a
/// category offers.
fn assemble(rows: Vec<TagRow>, pairs: Vec<(u32, u32)>) -> StickMyTags {
    let (mut categories, tags): (Vec<TagRow>, Vec<TagRow>) = rows.into_iter().partition(|r| r.3);
    categories.sort_by_key(|r| (r.1, r.0));
    let categories: Vec<StickTagCategory> = categories
        .into_iter()
        .map(|(id, _, name, _, _)| {
            let mut under: Vec<&TagRow> = tags.iter().filter(|t| t.4 == id).collect();
            under.sort_by_key(|t| (t.1, t.0));
            StickTagCategory {
                id,
                name,
                tags: under
                    .into_iter()
                    .map(|t| StickTag {
                        id: t.0,
                        name: t.2.clone(),
                    })
                    .collect(),
            }
        })
        .collect();
    // Each offered tag's place, for ordering a track's tags.
    let place: BTreeMap<u32, usize> = categories
        .iter()
        .flat_map(|c| c.tags.iter().map(|t| t.id))
        .enumerate()
        .map(|(at, id)| (id, at))
        .collect();
    let mut tracks: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (track, tag) in pairs {
        if place.contains_key(&tag) {
            tracks.entry(track).or_default().push(tag);
        }
    }
    for held in tracks.values_mut() {
        held.sort_by_key(|id| place.get(id).copied().unwrap_or(usize::MAX));
        held.dedup();
    }
    StickMyTags { categories, tracks }
}
