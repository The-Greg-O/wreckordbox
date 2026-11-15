//! Rebuilding the Library from a Stick (wbx-b1d): every playlist and folder,
//! rating, colour, comment and MyTag a Stick's library holds, each track
//! linked to its file in a music folder on this computer rather than copied
//! from the Stick.
//!
//! For a Stick that rekordbox wrote on another computer: its tracks name
//! that computer's Library and paths, so neither Import Playlist nor
//! Round-trip (which match by this Library's identity) can find them. Here
//! a track is found by its file name and size in the music folder
//! ([`matching`]); an ambiguous or missing match is reported by name and
//! never guessed.
//!
//! The Stick is only read, by [`wreckordbox_stick_outline::read`] (no
//! recovery, no lock made, an unfinished write refused) and a `stat` of its
//! audio files. A dry run writes nothing anywhere. Applying is one import
//! Run under the caller's source and one edit through the Library's single
//! edit path, refused while Library Protection is on.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use rbl_db::DbError;
use rbl_export::device_library::{Format, Library};
use rbl_library::Touched;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wreckordbox_stick_outline::StickMyTags;

use crate::library::OpenLibrary;
use crate::protocol::{codes as shared_codes, LibraryChanged, ServiceError};

/// The names of the commands this module answers.
pub mod commands {
    /// Takes a [`Rebuild`](super::Rebuild), answers a
    /// [`Rebuilt`](super::Rebuilt).
    pub const LIBRARY_REBUILD_FROM_STICK: &str = "library.rebuild_from_stick";
}

/// The error codes this command adds.
pub mod codes {
    /// The Stick could not be read: no rekordbox library on it, an
    /// unfinished write, or a file that cannot be parsed.
    pub const STICK_UNREADABLE: &str = "stick_unreadable";
    /// The music folder is not a folder that can be read.
    pub const MUSIC_FOLDER_UNREADABLE: &str = "music_folder_unreadable";
}

/// What cannot be carried, in every answer.
const NOT_CARRIED: [&str; 3] = [
    "cues, loops and beat grids of a track already analysed in this Library: it keeps its own analysis files (replacing them needs the window's analysis journal)",
    "play history: not carried yet",
    "artwork and date added: not carried",
];

pub(crate) fn answers(command: &str) -> bool {
    command == commands::LIBRARY_REBUILD_FROM_STICK
}

/// The arguments of `library.rebuild_from_stick`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rebuild {
    /// Where the Stick is mounted.
    pub stick: PathBuf,
    /// The folder holding the audio files the tracks are linked to.
    pub music: PathBuf,
    /// Only say what would be done.
    #[serde(default)]
    pub dry_run: bool,
}

/// One value of a matched track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueOutcome {
    /// `rating`, `colour`, `comment`, `my_tags`, `bpm` or `key`.
    pub value: String,
    /// `set` (a track this rebuild adds), `kept` (already the Stick's) or
    /// `replaced` (the Library's value gives way to the Stick's); for
    /// `my_tags` on a track already there, `added` or `kept`.
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Value>,
    pub to: Value,
}

/// A Stick track matched to a file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchedTrack {
    pub title: String,
    pub artist: String,
    /// Where the Stick keeps it, volume-relative.
    pub stick_path: String,
    /// The file in the music folder it is linked to.
    pub file: String,
    /// The collection's track already at that file, if any.
    pub existing: Option<String>,
    /// The collection's track after applying; `null` in a dry run of a new one.
    pub track: Option<String>,
    pub values: Vec<ValueOutcome>,
}

/// A Stick track with no single file to link to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmatchedTrack {
    pub title: String,
    pub artist: String,
    pub stick_path: String,
    /// `no_file`, `ambiguous` or `different_size`.
    pub reason: String,
    /// The files of that name it could be.
    pub candidates: Vec<String>,
}

/// A playlist or folder of the Stick, in tree order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistOutcome {
    /// Its names from the top, joined with `/`.
    pub path: String,
    pub folder: bool,
    /// `create`, `reuse` (a folder of that name already there) or `replace`
    /// (a playlist of that name already there gets the Stick's entries).
    pub action: String,
    /// Its entries that are carried.
    pub tracks: usize,
    /// The titles of its entries that are not (no match).
    pub left_out: Vec<String>,
    /// Its identifier in the Library after applying.
    pub id: Option<String>,
}

/// One tag of a Stick category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagOutcome {
    pub name: String,
    /// `exists`, `create`, or `not_carried` (its category is not here).
    pub action: String,
    /// The matched tracks carrying it.
    pub tracks: usize,
    pub id: Option<String>,
}

/// One MyTag category of the Stick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryOutcome {
    /// The Stick's name for it.
    pub name: String,
    /// Its slot: its place among the Stick's categories, from 1. rekordbox
    /// has fixed slots whose names the DJ can change, so a Stick category
    /// goes to the Library's category in the same slot.
    pub slot: usize,
    /// The Library's category in that slot; `null` when the Library has
    /// fewer slots (rekordbox fixes them, so none is made).
    pub library_category: Option<String>,
    /// That category's name before the rebuild.
    pub library_name: Option<String>,
    /// Whether the Library's category is renamed to the Stick's name.
    pub rename: bool,
    pub tags: Vec<TagOutcome>,
}

/// What `library.rebuild_from_stick` answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rebuilt {
    pub dry_run: bool,
    pub stick: String,
    pub music: String,
    /// `device_library` or `one_library`: the Stick's library read.
    pub format: String,
    pub matched: Vec<MatchedTrack>,
    pub unmatched: Vec<UnmatchedTrack>,
    pub playlists: Vec<PlaylistOutcome>,
    pub my_tags: Vec<CategoryOutcome>,
    /// For each kind of value, how many matched tracks it is carried for.
    pub values: BTreeMap<String, usize>,
    /// What the rebuild cannot carry, in words.
    pub not_carried: Vec<String>,
    /// The Library's generation after applying.
    pub generation: Option<u32>,
    /// Why the Ledger could not record all of it, when it could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_warning: Option<String>,
}

/// The rule that links a Stick track to a file of the music folder.
pub mod matching {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    /// The music folder's audio files by file name, case folded, each with
    /// its size. Links are not followed.
    #[derive(Debug, Default)]
    pub struct MusicFolder {
        by_name: HashMap<String, Vec<(PathBuf, u64)>>,
    }

    /// Where a Stick track's file is.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Found {
        File(PathBuf),
        NoFile,
        /// Several files it could be, none or several of them the size of
        /// the Stick's copy.
        Ambiguous(Vec<PathBuf>),
        /// One file of that name, but not the size of the Stick's copy.
        DifferentSize(Vec<PathBuf>),
    }

    impl MusicFolder {
        /// Reads `root` and every folder below it.
        ///
        /// # Errors
        ///
        /// Why `root` cannot be listed (a folder below it that cannot be is
        /// passed over).
        pub fn read(root: &Path) -> std::io::Result<Self> {
            let mut folder = Self::default();
            let mut stack = vec![root.to_path_buf()];
            let mut first = true;
            while let Some(dir) = stack.pop() {
                let entries = match std::fs::read_dir(&dir) {
                    Ok(entries) => entries,
                    Err(error) if first => return Err(error),
                    Err(_) => continue,
                };
                first = false;
                for entry in entries.flatten() {
                    let Ok(kind) = entry.file_type() else {
                        continue;
                    };
                    let path = entry.path();
                    if kind.is_dir() {
                        stack.push(path);
                    } else if kind.is_file() && rbl_db::import::is_audio(&path) {
                        let size = entry.metadata().map_or(0, |m| m.len());
                        let name = entry.file_name().to_string_lossy().to_lowercase();
                        folder.by_name.entry(name).or_default().push((path, size));
                    }
                }
            }
            for files in folder.by_name.values_mut() {
                files.sort();
            }
            Ok(folder)
        }

        /// The file a Stick track recorded as `stick_path`, whose copy on the
        /// Stick is `stick_size` bytes (`None`: the Stick has no copy), is:
        /// the one file of the same name (case folded) whose size is the
        /// Stick copy's; without a copy to compare, the one file of that name.
        #[must_use]
        pub fn find(&self, stick_path: &str, stick_size: Option<u64>) -> Found {
            let name = file_name(stick_path).to_lowercase();
            let Some(files) = self.by_name.get(&name).filter(|f| !f.is_empty()) else {
                return Found::NoFile;
            };
            let paths = |files: &[&(PathBuf, u64)]| files.iter().map(|(p, _)| p.clone()).collect();
            let all: Vec<&(PathBuf, u64)> = files.iter().collect();
            let Some(size) = stick_size else {
                return if let [(only, _)] = files.as_slice() {
                    Found::File(only.clone())
                } else {
                    Found::Ambiguous(paths(&all))
                };
            };
            let same: Vec<&(PathBuf, u64)> = files.iter().filter(|(_, s)| *s == size).collect();
            match same.as_slice() {
                [(only, _)] => Found::File(only.clone()),
                [] if all.len() == 1 => Found::DifferentSize(paths(&all)),
                [] => Found::Ambiguous(paths(&all)),
                _ => Found::Ambiguous(paths(&same)),
            }
        }
    }

    /// The last component of a Stick's recorded path.
    #[must_use]
    pub fn file_name(stick_path: &str) -> String {
        let clean = stick_path.replace('\\', "/");
        clean.rsplit('/').next().unwrap_or_default().to_owned()
    }
}

/// What the Stick holds, read in one read.
struct StickState {
    format: Format,
    library: Library,
    my_tags: StickMyTags,
    /// Each track's audio file size on the Stick, by its id; absent when the
    /// Stick holds no copy (or it lies outside the Stick).
    sizes: HashMap<u32, u64>,
}

fn read_stick(mount: &Path) -> Result<StickState, ServiceError> {
    let unreadable = |e: &dyn std::fmt::Display| {
        ServiceError::new(
            codes::STICK_UNREADABLE,
            format!("the Stick at {} cannot be read: {e}", mount.display()),
        )
    };
    wreckordbox_stick_outline::read(mount, |stick| {
        let formats = rbl_export::device_library::formats(stick.root());
        // OneLibrary is what rekordbox 7 reads first; the Device Library
        // otherwise.
        let format = if formats.contains(&Format::OneLibrary) {
            Format::OneLibrary
        } else if formats.contains(&Format::DeviceLibrary) {
            Format::DeviceLibrary
        } else {
            return Err(wreckordbox_stick_outline::ReadError::Unreadable(
                "it holds no rekordbox library".into(),
            ));
        };
        let library = stick.library(format)?;
        let my_tags = stick.my_tags(format)?;
        let root = stick.root();
        let mut sizes = HashMap::new();
        for track in &library.tracks {
            let at = root.join(track.path.replace('\\', "/").trim_start_matches('/'));
            // Only a stat, and only of a file that is really on the Stick.
            if let Ok(real) = at.canonicalize() {
                if real.starts_with(root) {
                    if let Ok(meta) = std::fs::metadata(&real) {
                        if meta.is_file() {
                            sizes.insert(track.id, meta.len());
                        }
                    }
                }
            }
        }
        Ok(StickState {
            format,
            library,
            my_tags,
            sizes,
        })
    })
    .map_err(|e| unreadable(&e))
}

/// A track of the collection, as far as the rebuild compares it.
#[derive(Debug, Clone)]
struct Existing {
    id: String,
    rating: u8,
    colour: u8,
    comment: String,
    tags: Vec<String>,
    /// It has analysis files or cues of its own, which the rebuild keeps.
    analysed: bool,
}

/// What the Library holds that the rebuild compares with.
struct LibraryState {
    existing: HashMap<PathBuf, Existing>,
    /// `(id, name, parent)` of each live category (`Attribute` 1 under root).
    categories: Vec<(String, String)>,
    /// Each category's tags: `(id, name)`.
    tags: HashMap<String, Vec<(String, String)>>,
    /// `(parent, name, folder)` -> id, for the plain playlists and folders.
    nodes: HashMap<(String, String, bool), String>,
}

fn read_library(library: &OpenLibrary, files: &[PathBuf]) -> Result<LibraryState, ServiceError> {
    library.with_db(|db| {
        let conn = db.connection();
        let failed = |e: rusqlite::Error| {
            ServiceError::new(shared_codes::LIBRARY_DAMAGED, format!("the Library cannot be read: {e}"))
        };
        let mut existing = HashMap::new();
        let mut at = conn
            .prepare(
                "SELECT ID, COALESCE(Rating, 0), COALESCE(CAST(NULLIF(ColorID, '') AS INTEGER), 0),
                        COALESCE(Commnt, ''),
                        COALESCE(AnalysisDataPath, '') <> ''
                          OR EXISTS (SELECT 1 FROM djmdCue
                                     WHERE ContentID = djmdContent.ID AND rb_local_deleted = 0)
                 FROM djmdContent WHERE FolderPath = ?1 AND rb_local_deleted = 0 ORDER BY ID LIMIT 1",
            )
            .map_err(failed)?;
        let mut tags_of = conn
            .prepare(
                "SELECT MyTagID FROM djmdSongMyTag WHERE ContentID = ?1 AND rb_local_deleted = 0
                 ORDER BY MyTagID",
            )
            .map_err(failed)?;
        for file in files {
            let row = at.query_row([file.to_string_lossy().as_ref()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, bool>(4)?,
                ))
            });
            let (id, rating, colour, comment, analysed) = match row {
                Ok(row) => row,
                Err(rusqlite::Error::QueryReturnedNoRows) => continue,
                Err(e) => return Err(failed(e)),
            };
            let tags = tags_of
                .query_map([&id], |r| r.get::<_, String>(0))
                .map_err(failed)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(failed)?;
            existing.insert(
                file.clone(),
                Existing {
                    id,
                    rating: u8::try_from(rating).unwrap_or(0),
                    colour: u8::try_from(colour).unwrap_or(0),
                    comment,
                    tags,
                    analysed,
                },
            );
        }
        let mut rows = conn
            .prepare(
                "SELECT ID, COALESCE(Name, ''), COALESCE(Attribute, 0), COALESCE(ParentID, '')
                 FROM djmdMyTag WHERE rb_local_deleted = 0 ORDER BY Seq, ID",
            )
            .map_err(failed)?;
        let tag_rows = rows
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))
            })
            .map_err(failed)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(failed)?;
        let categories: Vec<(String, String)> = tag_rows
            .iter()
            .filter(|(_, _, attribute, parent)| *attribute == 1 && parent == "root")
            .map(|(id, name, _, _)| (id.clone(), name.clone()))
            .collect();
        let mut tags: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for (id, name, attribute, parent) in &tag_rows {
            if *attribute == 0 {
                tags.entry(parent.clone()).or_default().push((id.clone(), name.clone()));
            }
        }
        let mut lists = conn
            .prepare(
                "SELECT ID, COALESCE(NULLIF(ParentID, ''), 'root'), COALESCE(Name, ''), COALESCE(Attribute, 0)
                 FROM djmdPlaylist WHERE rb_local_deleted = 0 ORDER BY Seq, ID",
            )
            .map_err(failed)?;
        let mut nodes = HashMap::new();
        for row in lists
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?))
            })
            .map_err(failed)?
        {
            let (id, parent, name, attribute) = row.map_err(failed)?;
            let folder = match attribute {
                a if a == rbl_db::write::ATTRIBUTE_FOLDER => true,
                a if a == rbl_db::write::ATTRIBUTE_PLAYLIST => false,
                _ => continue,
            };
            nodes.entry((parent, name, folder)).or_insert(id);
        }
        Ok(LibraryState { existing, categories, tags, nodes })
    })
}

/// A Stick tag's place in the Library.
#[derive(Debug, Clone)]
enum TagTarget {
    Exists(String),
    /// To make in this category, under this name.
    Create {
        category: String,
        name: String,
    },
}

/// One matched track, planned.
struct PlannedTrack {
    stick_id: u32,
    file: PathBuf,
    existing: Option<Existing>,
    rating: u8,
    colour: u8,
    comment: String,
    bpm_x100: u32,
    key: String,
    /// The Stick tag ids to carry.
    tags: Vec<u32>,
    /// The Stick's analysis `.DAT`, volume-relative, to carry (its cues,
    /// loops and beat grid, with its `.EXT` and `.2EX`); `None` when the
    /// Stick has none or the track keeps its own.
    analysis: Option<String>,
}

/// One node, planned: the Stick node id, its parent's Stick id (0 at the
/// top), and the Library node to reuse.
struct PlannedNode {
    stick_id: u32,
    stick_parent: u32,
    name: String,
    folder: bool,
    reuse: Option<String>,
    /// Stick track ids of the entries carried, in order.
    entries: Vec<u32>,
}

struct Plan {
    /// Where the Stick is mounted, to read its analysis files from.
    stick: PathBuf,
    /// The Library's share root, where analysis files go.
    share_root: PathBuf,
    /// `(Library category, the Stick's name for its slot)`.
    renames: Vec<(String, String)>,
    tracks: Vec<PlannedTrack>,
    nodes: Vec<PlannedNode>,
    tags: HashMap<u32, TagTarget>,
    answer: Rebuilt,
}

fn plan(
    request: &Rebuild,
    music: &Path,
    stick: &StickState,
    folder: &matching::MusicFolder,
    library: &OpenLibrary,
) -> Result<Plan, ServiceError> {
    let mut matched_files: HashMap<u32, PathBuf> = HashMap::new();
    let mut unmatched = Vec::new();
    for track in &stick.library.tracks {
        let shown = |p: &[PathBuf]| p.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let (reason, candidates) =
            match folder.find(&track.path, stick.sizes.get(&track.id).copied()) {
                matching::Found::File(file) => {
                    matched_files.insert(track.id, file);
                    continue;
                }
                matching::Found::NoFile => ("no_file", Vec::new()),
                matching::Found::Ambiguous(files) => ("ambiguous", shown(&files)),
                matching::Found::DifferentSize(files) => ("different_size", shown(&files)),
            };
        unmatched.push(UnmatchedTrack {
            title: track.title.clone(),
            artist: track.artist.clone(),
            stick_path: track.path.clone(),
            reason: reason.into(),
            candidates,
        });
    }
    let files: Vec<PathBuf> = matched_files.values().cloned().collect();
    let state = read_library(library, &files)?;

    // MyTags: a Stick category to the Library's in the same slot, renamed
    // to the Stick's name when the two differ.
    let mut tags = HashMap::new();
    let mut renames = Vec::new();
    let mut tag_tracks: HashMap<u32, usize> = HashMap::new();
    for (track, ids) in &stick.my_tags.tracks {
        if matched_files.contains_key(track) {
            for id in ids {
                *tag_tracks.entry(*id).or_default() += 1;
            }
        }
    }
    let mut categories = Vec::new();
    for (slot, category) in stick.my_tags.categories.iter().enumerate() {
        let in_slot = state.categories.get(slot).cloned();
        let here = in_slot.as_ref().map(|(id, _)| id.clone());
        let rename = in_slot
            .as_ref()
            .is_some_and(|(_, name)| *name != category.name);
        if let (true, Some((id, _))) = (rename, &in_slot) {
            renames.push((id.clone(), category.name.clone()));
        }
        let mut outcomes = Vec::new();
        for tag in &category.tags {
            let (action, id) = match &here {
                None => ("not_carried", None),
                Some(cat) => {
                    let found = state
                        .tags
                        .get(cat)
                        .and_then(|t| {
                            t.iter()
                                .find(|(_, name)| name.eq_ignore_ascii_case(&tag.name))
                        })
                        .map(|(id, _)| id.clone());
                    if let Some(id) = found {
                        tags.insert(tag.id, TagTarget::Exists(id.clone()));
                        ("exists", Some(id))
                    } else {
                        tags.insert(
                            tag.id,
                            TagTarget::Create {
                                category: cat.clone(),
                                name: tag.name.clone(),
                            },
                        );
                        ("create", None)
                    }
                }
            };
            outcomes.push(TagOutcome {
                name: tag.name.clone(),
                action: action.into(),
                tracks: tag_tracks.get(&tag.id).copied().unwrap_or(0),
                id,
            });
        }
        categories.push(CategoryOutcome {
            name: category.name.clone(),
            slot: slot + 1,
            library_category: here,
            library_name: in_slot.map(|(_, name)| name),
            rename,
            tags: outcomes,
        });
    }
    // A name the renames would give twice, or that a category left as it is
    // already has, cannot be given: say so before anything is written.
    let kept: Vec<&String> = state
        .categories
        .iter()
        .filter(|(id, _)| !renames.iter().any(|(renamed, _)| renamed == id))
        .map(|(_, name)| name)
        .collect();
    for (at, (_, name)) in renames.iter().enumerate() {
        if kept.contains(&name) || renames[..at].iter().any(|(_, earlier)| earlier == name) {
            return Err(ServiceError::new(
                crate::mytag_edits::codes::MY_TAG_NAME_TAKEN,
                format!("the MyTag category \"{name}\" cannot be given to its slot: another category of this Library has that name"),
            ));
        }
    }
    let tag_name = |id: u32| -> Option<String> {
        stick
            .my_tags
            .categories
            .iter()
            .flat_map(|c| &c.tags)
            .find(|t| t.id == id)
            .map(|t| t.name.clone())
    };

    // Tracks and their values.
    let mut planned = Vec::new();
    let mut matched = Vec::new();
    let mut values: BTreeMap<String, usize> = BTreeMap::new();
    for track in &stick.library.tracks {
        let Some(file) = matched_files.get(&track.id) else {
            continue;
        };
        let existing = state.existing.get(file).cloned();
        let carried_tags: Vec<u32> = stick
            .my_tags
            .tracks
            .get(&track.id)
            .map(|ids| {
                ids.iter()
                    .copied()
                    .filter(|id| tags.contains_key(id))
                    .collect()
            })
            .unwrap_or_default();
        let mut outcomes = Vec::new();
        let mut compare = |value: &str, stick_value: Value, held: bool, before: Option<Value>| {
            match before {
                None if held => outcomes.push(ValueOutcome {
                    value: value.into(),
                    outcome: "set".into(),
                    from: None,
                    to: stick_value,
                }),
                None => {}
                Some(before) if before == stick_value => {
                    outcomes.push(ValueOutcome {
                        value: value.into(),
                        outcome: "kept".into(),
                        from: None,
                        to: stick_value,
                    });
                }
                Some(before) => outcomes.push(ValueOutcome {
                    value: value.into(),
                    outcome: "replaced".into(),
                    from: Some(before),
                    to: stick_value,
                }),
            }
            if held {
                *values.entry(value.into()).or_default() += 1;
            }
        };
        let rating = track.rating.min(5);
        compare(
            "rating",
            json!(rating),
            rating > 0,
            existing.as_ref().map(|e| json!(e.rating)),
        );
        compare(
            "colour",
            json!(track.color),
            track.color > 0,
            existing.as_ref().map(|e| json!(e.colour)),
        );
        compare(
            "comment",
            json!(track.comment),
            !track.comment.is_empty(),
            existing.as_ref().map(|e| json!(e.comment)),
        );
        if existing.is_none() {
            compare(
                "bpm",
                json!(f64::from(track.bpm_x100) / 100.0),
                track.bpm_x100 > 0,
                None,
            );
            compare("key", json!(track.key), !track.key.is_empty(), None);
        }
        // Cues, loops and the beat grid: the Stick's analysis files, for a
        // track with none of its own here.
        let analysis =
            (!track.analysis_path.trim().is_empty()).then(|| track.analysis_path.clone());
        let keeps_own = existing.as_ref().is_some_and(|e| e.analysed);
        if analysis.is_some() {
            if keeps_own {
                outcomes.push(ValueOutcome {
                    value: "analysis".into(),
                    outcome: "kept".into(),
                    from: None,
                    to: json!("this Library's own cues, loops and beat grid"),
                });
            } else {
                *values.entry("analysis".into()).or_default() += 1;
                outcomes.push(ValueOutcome {
                    value: "analysis".into(),
                    outcome: "set".into(),
                    from: None,
                    to: json!("the Stick's cues, loops and beat grid"),
                });
            }
        }
        let analysis = analysis.filter(|_| !keeps_own);
        let names: Vec<String> = carried_tags.iter().filter_map(|id| tag_name(*id)).collect();
        if !carried_tags.is_empty() {
            *values.entry("my_tags".into()).or_default() += 1;
            let added = existing.as_ref().map(|e| {
                carried_tags.iter().any(|id| match &tags[id] {
                    TagTarget::Exists(lib) => !e.tags.contains(lib),
                    TagTarget::Create { .. } => true,
                })
            });
            let outcome = match added {
                None => "set",
                Some(true) => "added",
                Some(false) => "kept",
            };
            outcomes.push(ValueOutcome {
                value: "my_tags".into(),
                outcome: outcome.into(),
                from: None,
                to: json!(names),
            });
        }
        matched.push(MatchedTrack {
            title: track.title.clone(),
            artist: track.artist.clone(),
            stick_path: track.path.clone(),
            file: file.to_string_lossy().into_owned(),
            existing: existing.as_ref().map(|e| e.id.clone()),
            track: existing.as_ref().map(|e| e.id.clone()),
            values: outcomes,
        });
        planned.push(PlannedTrack {
            stick_id: track.id,
            file: file.clone(),
            existing,
            rating,
            colour: track.color,
            comment: track.comment.clone(),
            bpm_x100: track.bpm_x100,
            key: track.key.clone(),
            tags: carried_tags,
            analysis,
        });
    }

    // The tree, in the Stick's order; a node is reused only under a reused
    // parent (a new folder holds nothing yet).
    let mut library_ids: HashMap<u32, Option<String>> =
        HashMap::from([(0, Some("root".to_owned()))]);
    let mut paths: HashMap<u32, String> = HashMap::new();
    let mut nodes = Vec::new();
    let mut playlists = Vec::new();
    for node in &stick.library.nodes {
        let parent_here = library_ids.get(&node.parent).cloned().flatten();
        let reuse = parent_here.and_then(|parent| {
            state
                .nodes
                .get(&(parent, node.name.clone(), node.folder))
                .cloned()
        });
        library_ids.insert(node.id, reuse.clone());
        let path = match paths.get(&node.parent) {
            Some(parent) => format!("{parent}/{}", node.name),
            None => node.name.clone(),
        };
        paths.insert(node.id, path.clone());
        let entries: Vec<u32> = node
            .tracks
            .iter()
            .copied()
            .filter(|t| matched_files.contains_key(t))
            .collect();
        let left_out = node
            .tracks
            .iter()
            .filter(|t| !matched_files.contains_key(t))
            .filter_map(|t| stick.library.track(*t).map(|t| t.title.clone()))
            .collect();
        let action = match (&reuse, node.folder) {
            (None, _) => "create",
            (Some(_), true) => "reuse",
            (Some(_), false) => "replace",
        };
        playlists.push(PlaylistOutcome {
            path,
            folder: node.folder,
            action: action.into(),
            tracks: entries.len(),
            left_out,
            id: reuse.clone(),
        });
        nodes.push(PlannedNode {
            stick_id: node.id,
            stick_parent: node.parent,
            name: node.name.clone(),
            folder: node.folder,
            reuse,
            entries,
        });
    }

    let mut not_carried: Vec<String> = categories
        .iter()
        .filter(|c| c.library_category.is_none())
        .map(|c| {
            format!(
                "MyTag category \"{}\" ({}): its slot ({}) is past this Library's last category, and rekordbox fixes the categories",
                c.name,
                c.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "),
                c.slot,
            )
        })
        .collect();
    not_carried.extend(NOT_CARRIED.iter().map(|s| (*s).to_owned()));

    Ok(Plan {
        stick: request.stick.clone(),
        share_root: library.share_root(),
        renames,
        tracks: planned,
        nodes,
        tags,
        answer: Rebuilt {
            dry_run: request.dry_run,
            stick: request.stick.to_string_lossy().into_owned(),
            music: music.to_string_lossy().into_owned(),
            format: match stick.format {
                Format::DeviceLibrary => "device_library",
                Format::OneLibrary => "one_library",
            }
            .into(),
            matched,
            unmatched,
            playlists,
            my_tags: categories,
            values,
            not_carried,
            generation: None,
            recording_warning: None,
        },
    })
}

/// The Library's identifiers the apply made, and its Ledger records.
struct Applied {
    /// Why a track's analysis could not be carried, by its Stick id.
    analysis_failed: HashMap<u32, String>,
    tracks: HashMap<u32, String>,
    nodes: HashMap<u32, String>,
    tags: HashMap<u32, String>,
    records: Vec<Value>,
}

/// Analysis files a track was given, and its cues.
struct Carried {
    relative: String,
    cues: Vec<rbl_anlz::CueEntry>,
    files: Vec<PathBuf>,
}

impl Carried {
    /// Takes the files away again, for an edit that failed after them.
    fn remove(&self) {
        for file in &self.files {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// Gives the track `id`, linked to `audio`, the Stick's analysis at `dat`
/// (volume-relative) and its `.EXT` and `.2EX`, as Import Playlist gives a
/// track it copies (`carry_stick_analysis` in the app): every section byte
/// for byte as on the Stick, the path inside each the share tree's, written
/// where rekordbox keeps the track's analysis. The cues are the `.EXT`'s
/// list when there is one (colours and comments), the `.DAT`'s otherwise.
///
/// Only for a track with no analysis of its own: the place is new (the
/// track's UUID-derived path), so nothing is overwritten and the window's
/// analysis journal, which exists to put overwritten files back, is not
/// needed. A crash between the files and the row leaves files no row names;
/// files already at the place are left alone and the track is reported.
/// The Stick is only read, one read per track, below its mount.
fn carry_analysis(
    writer: &rbl_db::write::Writer,
    plan: &Plan,
    dat: &str,
    audio: &Path,
    id: &str,
) -> Result<Carried, String> {
    let dat_relative = Path::new(dat).to_path_buf();
    let (dat, ext, two_ex) = wreckordbox_stick_outline::read(&plan.stick, |stick| {
        let read = |relative: &Path| -> Result<Option<rbl_anlz::Anlz>, wreckordbox_stick_outline::ReadError> {
            let text = relative.to_string_lossy().replace('\\', "/");
            let at = stick.root().join(text.trim_start_matches('/'));
            let Ok(real) = at.canonicalize() else { return Ok(None) };
            if !real.starts_with(stick.root()) || !real.is_file() {
                return Ok(None);
            }
            rbl_anlz::Anlz::read(&real).map(Some).map_err(|e| {
                wreckordbox_stick_outline::ReadError::Unreadable(format!("{}: {e}", relative.display()))
            })
        };
        Ok((
            read(&dat_relative)?,
            read(&dat_relative.with_extension("EXT"))?,
            read(&dat_relative.with_extension("2EX"))?,
        ))
    })
    .map_err(|e| e.to_string())?;
    let Some(dat) = dat else {
        return Err(format!(
            "its analysis file {} is not on the Stick",
            dat_relative.display()
        ));
    };
    let relative = writer
        .analysis_data_path_for(id)
        .map_err(|e| e.to_string())?;
    let target = rbl_anlz::resolve(&plan.share_root, &relative);
    let ppth = rbl_anlz::share_tree_ppth(&audio.to_string_lossy());
    let mut files = vec![(target.clone(), dat.with_path(&ppth))];
    for (extension, anlz) in [("EXT", &ext), ("2EX", &two_ex)] {
        if let Some(anlz) = anlz {
            files.push((rbl_anlz::sibling(&target, extension), anlz.with_path(&ppth)));
        }
    }
    if let Some((taken, _)) = files.iter().find(|(path, _)| path.exists()) {
        return Err(format!(
            "analysis files are already at {}; they were left as they are",
            taken.display()
        ));
    }
    let mut carried = Carried {
        relative,
        cues: ext
            .as_ref()
            .map_or_else(|| dat.legacy_cue_entries(), rbl_anlz::Anlz::cue_entries),
        files: Vec::new(),
    };
    for (path, bytes) in &files {
        if let Err(e) = write_new(path, bytes) {
            carried.remove();
            return Err(format!("{} could not be written: {e}", path.display()));
        }
        carried.files.push(path.clone());
    }
    Ok(carried)
}

/// Writes a file that does not exist yet, whole or not at all.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("rebuild-partial");
    let mut file = std::fs::File::create(&partial)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&partial, path)
}

/// Every write of the rebuild, inside the one edit.
fn apply(writer: &mut rbl_db::write::Writer, plan: &Plan) -> Result<Applied, DbError> {
    let mut records = Vec::new();
    let mut analysis_failed = HashMap::new();
    // Each renamed category under a name no other has first, so that slots
    // trading names never clash; then under the Stick's name.
    for (category, _) in &plan.renames {
        writer.rename_my_tag(category, &format!("rebuild-from-stick {category}"))?;
    }
    for (category, name) in &plan.renames {
        writer.rename_my_tag(category, name)?;
        records.push(
            json!({"operation": "rebuild_from_stick", "my_tag_category": category, "name": name}),
        );
    }
    let mut tags = HashMap::new();
    for (stick_id, target) in &plan.tags {
        let id = match target {
            TagTarget::Exists(id) => id.clone(),
            TagTarget::Create { category, name } => {
                // As the window makes one: the stock name, then the rename.
                let made = writer.create_my_tag(category)?;
                writer.rename_my_tag(&made.id, name)?;
                records.push(json!({"operation": "rebuild_from_stick", "my_tag": made.id, "name": name, "category": category}));
                made.id
            }
        };
        tags.insert(*stick_id, id);
    }
    let mut tracks = HashMap::new();
    let mut by_file: HashMap<PathBuf, String> = HashMap::new();
    for track in &plan.tracks {
        let (id, added) = match (&track.existing, by_file.get(&track.file)) {
            (Some(existing), _) => (existing.id.clone(), false),
            (None, Some(id)) => (id.clone(), false),
            (None, None) => match writer.track_id_at(&track.file)? {
                Some(id) => (id, false),
                None => (writer.import_file(&track.file)?, true),
            },
        };
        by_file.insert(track.file.clone(), id.clone());
        let mut changed = Vec::new();
        let carried = match &track.analysis {
            Some(dat) => match carry_analysis(writer, plan, dat, &track.file, &id) {
                Ok(carried) => Some(carried),
                Err(reason) => {
                    changed.push(json!({"field": "analysis", "failed": reason}));
                    analysis_failed.insert(track.stick_id, reason);
                    None
                }
            },
            None => None,
        };
        // The Stick's tempo and key go with its grid, or to a track that
        // has neither yet.
        if added || carried.is_some() {
            if !track.key.is_empty() {
                writer.ensure_detected_key(&track.key)?;
            }
            let key = Some(track.key.as_str()).filter(|k| !k.is_empty());
            let written = writer.import_device_analysis(
                &id,
                &rbl_db::write::DeviceAnalysis {
                    bpm_x100: track.bpm_x100,
                    key,
                    analysis_data_path: carried.as_ref().map(|c| c.relative.as_str()),
                    cues: carried.as_ref().map_or(&[], |c| c.cues.as_slice()),
                },
            );
            if let Err(error) = written {
                if let Some(carried) = &carried {
                    carried.remove();
                }
                return Err(error);
            }
            if let Some(carried) = &carried {
                changed.push(json!({"field": "analysis", "path": carried.relative, "cues": carried.cues.len()}));
            }
        }
        let before = track.existing.as_ref();
        if before.map_or(track.rating > 0, |e| e.rating != track.rating) {
            writer.set_rating(&id, track.rating)?;
            changed.push(
                json!({"field": "rating", "from": before.map(|e| e.rating), "to": track.rating}),
            );
        }
        if before.map_or(track.colour > 0, |e| e.colour != track.colour) {
            let colour = (track.colour > 0).then(|| track.colour.to_string());
            writer.set_color(&id, colour.as_deref())?;
            changed.push(
                json!({"field": "colour", "from": before.map(|e| e.colour), "to": track.colour}),
            );
        }
        if before.map_or(!track.comment.is_empty(), |e| e.comment != track.comment) {
            writer.set_comment(&id, &track.comment)?;
            changed.push(json!({"field": "comment", "from": before.map(|e| e.comment.clone()), "to": track.comment}));
        }
        if !track.tags.is_empty() {
            let mut all: Vec<String> = before.map(|e| e.tags.clone()).unwrap_or_default();
            let start = all.len();
            for stick_tag in &track.tags {
                let tag = &tags[stick_tag];
                if !all.contains(tag) {
                    all.push(tag.clone());
                }
            }
            if all.len() > start {
                writer.set_my_tags(&id, &all)?;
                changed.push(json!({"field": "my_tags", "added": &all[start..]}));
            }
        }
        records.push(json!({
            "operation": "rebuild_from_stick",
            "track": id,
            "file": track.file,
            "added": added,
            "changed": changed,
        }));
        tracks.insert(track.stick_id, id);
    }
    let mut nodes: HashMap<u32, String> = HashMap::new();
    for node in &plan.nodes {
        let parent = if node.stick_parent == 0 {
            "root".to_owned()
        } else {
            nodes
                .get(&node.stick_parent)
                .cloned()
                .unwrap_or_else(|| "root".to_owned())
        };
        let id = match &node.reuse {
            Some(id) => id.clone(),
            None if node.folder => writer.create_folder(&node.name, &parent)?,
            None => writer.create_playlist(&node.name, &parent)?,
        };
        if !node.folder {
            let contents: Vec<String> = node
                .entries
                .iter()
                .filter_map(|t| tracks.get(t).cloned())
                .collect();
            writer.set_tracks(&id, &contents)?;
        }
        records.push(json!({
            "operation": "rebuild_from_stick",
            "playlist": id,
            "name": node.name,
            "folder": node.folder,
            "made": node.reuse.is_none(),
            "tracks": node.entries.len(),
        }));
        nodes.insert(node.stick_id, id);
    }
    Ok(Applied {
        analysis_failed,
        tracks,
        nodes,
        tags,
        records,
    })
}

/// Answers `library.rebuild_from_stick`, and what to tell the hosting
/// window when the Library changed.
pub(crate) fn answer(
    library: &OpenLibrary,
    args: &Value,
) -> Result<(Value, Option<LibraryChanged>), ServiceError> {
    let request: Rebuild = serde_json::from_value(args.clone()).map_err(|e| {
        ServiceError::new(
            shared_codes::BAD_REQUEST,
            format!("library.rebuild_from_stick: {e}"),
        )
    })?;
    let music = request
        .music
        .canonicalize()
        .ok()
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            ServiceError::new(
                codes::MUSIC_FOLDER_UNREADABLE,
                format!(
                    "{} is not a folder that can be read",
                    request.music.display()
                ),
            )
        })?;
    let folder = matching::MusicFolder::read(&music).map_err(|e| {
        ServiceError::new(
            codes::MUSIC_FOLDER_UNREADABLE,
            format!("{} cannot be read: {e}", music.display()),
        )
    })?;
    let stick = read_stick(&request.stick)?;
    let mut plan = plan(&request, &music, &stick, &folder, library)?;
    if request.dry_run {
        return Ok((to_value(&plan.answer)?, None));
    }
    // Refused before anything is begun; the edit path refuses again under
    // its gate.
    if library.protection().protect_library {
        return Err(ServiceError::new(
            shared_codes::LIBRARY_PROTECTED,
            rbl_library::protection::MESSAGE,
        ));
    }
    // One import Run under the caller's source, one edit.
    let operation = library.import_operation();
    let capacity = plan.tags.len() + plan.tracks.len() + plan.nodes.len();
    let result = operation.run_many(
        json!({"operation": "rebuild_from_stick", "stick": request.stick, "music": music}),
        capacity,
        || library.edit(Touched::Tracks, |writer| apply(writer, &plan)),
        |(applied, _)| applied.records.clone(),
    );
    let warning = operation.finish();
    let (applied, changed) = result?;
    for track in &mut plan.answer.matched {
        if let Some(found) = stick
            .library
            .tracks
            .iter()
            .find(|t| t.path == track.stick_path)
            .and_then(|t| applied.tracks.get(&t.id))
        {
            track.track = Some(found.clone());
        }
    }
    for (track, planned) in plan.answer.matched.iter_mut().zip(&plan.tracks) {
        if let Some(reason) = applied.analysis_failed.get(&planned.stick_id) {
            for value in track.values.iter_mut().filter(|v| v.value == "analysis") {
                value.outcome = "failed".into();
                value.to = json!(reason);
            }
        }
    }
    for (outcome, node) in plan.answer.playlists.iter_mut().zip(&plan.nodes) {
        outcome.id = applied.nodes.get(&node.stick_id).cloned();
    }
    for category in &mut plan.answer.my_tags {
        let stick_category = stick
            .my_tags
            .categories
            .iter()
            .find(|c| c.name == category.name);
        for (outcome, tag) in category
            .tags
            .iter_mut()
            .zip(stick_category.into_iter().flat_map(|c| &c.tags))
        {
            if let Some(id) = applied.tags.get(&tag.id) {
                outcome.id = Some(id.clone());
            }
        }
    }
    plan.answer.generation = Some(changed.generation);
    plan.answer.recording_warning = warning;
    Ok((to_value(&plan.answer)?, Some(changed)))
}

fn to_value(answer: &Rebuilt) -> Result<Value, ServiceError> {
    serde_json::to_value(answer).map_err(|e| {
        ServiceError::new(
            shared_codes::SERVICE_FAILED,
            format!("could not answer: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::matching::{Found, MusicFolder};

    fn folder(files: &[(&str, usize)]) -> (tempfile::TempDir, MusicFolder) {
        let dir = tempfile::tempdir().unwrap();
        for (name, size) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, vec![0_u8; *size]).unwrap();
        }
        let read = MusicFolder::read(dir.path()).unwrap();
        (dir, read)
    }

    #[test]
    fn a_file_is_found_by_name_and_size_and_never_guessed() {
        let (dir, music) = folder(&[
            ("x/One.mp3", 10),
            ("Two.mp3", 5),
            ("a/Two.MP3", 6),
            ("b/Three.mp3", 7),
            ("c/three.mp3", 7),
            ("notes.txt", 1),
        ]);
        let at = |p: &str| dir.path().join(p);
        assert_eq!(
            music.find("/Contents/A/B/One.mp3", Some(10)),
            Found::File(at("x/One.mp3"))
        );
        assert_eq!(
            music.find("/Contents/A/B/one.MP3", None),
            Found::File(at("x/One.mp3"))
        );
        assert_eq!(
            music.find("/Contents/A/B/One.mp3", Some(11)),
            Found::DifferentSize(vec![at("x/One.mp3")])
        );
        assert_eq!(
            music.find("/Contents/Two.mp3", Some(6)),
            Found::File(at("a/Two.MP3"))
        );
        assert_eq!(
            music.find("/Contents/Two.mp3", None),
            Found::Ambiguous(vec![at("Two.mp3"), at("a/Two.MP3")])
        );
        assert_eq!(
            music.find("/Contents/Two.mp3", Some(9)),
            Found::Ambiguous(vec![at("Two.mp3"), at("a/Two.MP3")])
        );
        assert_eq!(
            music.find("/Contents/Three.mp3", Some(7)),
            Found::Ambiguous(vec![at("b/Three.mp3"), at("c/three.mp3")])
        );
        assert_eq!(music.find("/Contents/notes.txt", Some(1)), Found::NoFile);
        assert_eq!(music.find("/Contents/Four.mp3", Some(1)), Found::NoFile);
    }
}
