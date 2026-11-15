//! Rebuilding the Library from a Stick (wbx-b1d): every playlist and folder,
//! rating, colour, comment and My Tag a Stick's library holds, each track
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

const fn yes() -> bool {
    true
}

pub(crate) fn answers(command: &str) -> bool {
    command == commands::LIBRARY_REBUILD_FROM_STICK
}

/// The arguments of `library.rebuild_from_stick`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the command's flat JSON arguments"
)]
pub struct Rebuild {
    /// Where the Stick is mounted.
    pub stick: PathBuf,
    /// The folder holding the audio files the tracks are linked to.
    pub music: PathBuf,
    /// Only say what would be done.
    #[serde(default)]
    pub dry_run: bool,
    /// Link a track to the one file of its name when no file has its size
    /// (a file retagged since the export).
    #[serde(default)]
    pub accept_retagged: bool,
    /// Link a track to the one file of its size whose name agrees only in
    /// part, or whose length does not agree.
    #[serde(default)]
    pub accept_uncertain: bool,
    /// Read each candidate's length from its header and compare it with
    /// the Stick's (default). Off for a music folder whose files are slow to
    /// open (a network mount): names and sizes alone are then compared.
    #[serde(default = "yes")]
    pub check_length: bool,
    /// Test mode only: stop the apply after this step, as a crash would.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_after: Option<String>,
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
    /// Linked by its name alone: the file's size differs from the Stick
    /// copy's (retagged since the export), accepted by the caller.
    #[serde(default)]
    pub retagged: bool,
    /// Linked by its size with its name agreeing only in part (or its
    /// length not agreeing), accepted by the caller.
    #[serde(default)]
    pub uncertain: bool,
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
    /// `no_file`; `ambiguous` (several files it could be); `uncertain` (one
    /// file of exactly its size whose name agrees only in part or whose
    /// length does not agree: pass `accept_uncertain` to link it);
    /// `name_disagrees` (one file of exactly its size, whose name does not
    /// start as the Stick's does); `different_size` (one file of its name,
    /// of another size: pass `accept_retagged` to link it).
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

/// One My Tag category of the Stick.
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
    /// The backup of the Library taken before applying (`null` in a dry run).
    #[serde(default)]
    pub backup: Option<String>,
    /// Whether everything was applied (a dry run: `true`).
    #[serde(default = "yes")]
    pub complete: bool,
    /// When the apply stopped part way: what was done and what to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<Incomplete>,
}

/// An apply that stopped part way. What it wrote is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incomplete {
    /// Why it stopped.
    pub error: String,
    /// The steps finished ([`STEPS`]), in order.
    pub done: Vec<String>,
    /// What to do now, in words.
    pub advice: String,
}

/// The rule that links a Stick track to a file of the music folder.
///
/// rekordbox copies each file byte for byte to the Stick but cleans up its
/// name (the characters FAT cannot hold) and may cut it short, so the size
/// is what really ties the two together and the name confirms it. In order:
///
/// 1. The files whose size is exactly the Stick copy's. Several:
///    ambiguous. One: taken when the WHOLE of the Stick's name (its stem,
///    compared as [`matching::comparable`] makes it, a name rekordbox cut to
///    41 characters and numbered `-N` taken as those 41) is where the file's
///    name starts, and, where the Stick's library records a length and the
///    file's header says its own, the two lengths agree within a second.
///    One whose name agrees only over its first [`matching::AGREEING_CHARACTERS`]
///    characters, or whose length does not agree: `uncertain`, taken only
///    when the caller accepts uncertain matches. One whose name does not
///    agree at all: reported, never taken.
/// 2. No file of that size: the files of the same name (case folded). One:
///    "same name, different size" (retagged since the export), linked only
///    when the caller accepts retagged files. Several: ambiguous.
/// 3. Nothing: no file.
///
/// Without a Stick copy to measure, only a file of the same name, alone,
/// is taken. macOS `._` `AppleDouble` files are never audio and are skipped.
pub mod matching {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use unicode_normalization::UnicodeNormalization;

    /// How many characters of the two names' stems must agree for a size
    /// match to be `uncertain` rather than not a match at all.
    pub const AGREEING_CHARACTERS: usize = 20;

    /// How far apart, in seconds, the Stick's length and the file's may be.
    pub const LENGTH_TOLERANCE_SECS: u32 = 1;

    /// The music folder's audio files, by size and by file name (case
    /// folded). Links are not followed; only names and sizes are read here.
    #[derive(Debug, Default)]
    pub struct MusicFolder {
        by_size: HashMap<u64, Vec<PathBuf>>,
        by_name: HashMap<String, Vec<PathBuf>>,
        /// How many audio files it holds.
        pub files: usize,
    }

    /// What the caller accepts besides a certain match.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Accept {
        /// The one file of the same name, of another size.
        pub retagged: bool,
        /// The one file of exactly the size whose name agrees only in part,
        /// or whose length does not agree.
        pub uncertain: bool,
        /// Read a candidate's length from its header to compare.
        pub check_length: bool,
    }

    /// Where a Stick track's file is.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Found {
        File(PathBuf),
        /// The one file of that name, of another size: retagged since the
        /// export, linked because the caller asked.
        Retagged(PathBuf),
        /// The one file of its size, its name agreeing only in part or its
        /// length not agreeing, linked because the caller asked.
        AcceptedUncertain(PathBuf),
        NoFile,
        /// Several files it could be.
        Ambiguous(Vec<PathBuf>),
        /// One file of exactly that size whose name does not agree.
        NameDisagrees(PathBuf),
        /// One file of exactly that size whose name agrees only in part, or
        /// whose length does not agree; not accepted.
        Uncertain(PathBuf),
        /// One file of that name, of another size, not accepted.
        DifferentSize(PathBuf),
    }

    /// Whether a name is a macOS `AppleDouble` companion, never audio.
    #[must_use]
    pub fn apple_double(name: &str) -> bool {
        name.starts_with("._")
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
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if apple_double(&name) {
                        continue;
                    }
                    let Ok(kind) = entry.file_type() else {
                        continue;
                    };
                    let path = entry.path();
                    if kind.is_dir() {
                        stack.push(path);
                    } else if kind.is_file() && rbl_db::import::is_audio(&path) {
                        let Ok(meta) = entry.metadata() else { continue };
                        folder.files += 1;
                        folder
                            .by_size
                            .entry(meta.len())
                            .or_default()
                            .push(path.clone());
                        folder
                            .by_name
                            .entry(name.to_lowercase())
                            .or_default()
                            .push(path);
                    }
                }
            }
            for files in folder
                .by_size
                .values_mut()
                .chain(folder.by_name.values_mut())
            {
                files.sort();
            }
            Ok(folder)
        }

        /// The file of a Stick track recorded as `stick_path`, whose copy on
        /// the Stick is `stick_size` bytes (`None`: the Stick has no copy)
        /// and whose library gives it `stick_secs` seconds (0: not recorded).
        #[must_use]
        pub fn find(
            &self,
            stick_path: &str,
            stick_size: Option<u64>,
            stick_secs: u32,
            accept: Accept,
        ) -> Found {
            let name = file_name(stick_path);
            let named: &[PathBuf] = self
                .by_name
                .get(&name.to_lowercase())
                .map_or(&[], Vec::as_slice);
            let Some(size) = stick_size else {
                return match named {
                    [] => Found::NoFile,
                    [only] => Found::File(only.clone()),
                    several => Found::Ambiguous(several.to_vec()),
                };
            };
            match self.by_size.get(&size).map_or(&[][..], Vec::as_slice) {
                [only] => {
                    let theirs = only
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let certain = whole_name_agrees(&name, &theirs)
                        && (!accept.check_length || lengths_agree(only, stick_secs));
                    return if certain {
                        Found::File(only.clone())
                    } else if !names_agree(&name, &theirs) {
                        Found::NameDisagrees(only.clone())
                    } else if accept.uncertain {
                        Found::AcceptedUncertain(only.clone())
                    } else {
                        Found::Uncertain(only.clone())
                    };
                }
                [] => {}
                several => return Found::Ambiguous(several.to_vec()),
            }
            match named {
                [] => Found::NoFile,
                [only] if accept.retagged => Found::Retagged(only.clone()),
                [only] => Found::DifferentSize(only.clone()),
                several => Found::Ambiguous(several.to_vec()),
            }
        }
    }

    /// Whether the file's length, read from its header, agrees with the
    /// Stick's within [`LENGTH_TOLERANCE_SECS`]. A length that is not
    /// recorded, or a header that cannot be read, does not disagree.
    fn lengths_agree(file: &Path, stick_secs: u32) -> bool {
        if stick_secs == 0 {
            return true;
        }
        match rbl_db::import::read_tags(file) {
            Ok(tags) if tags.duration_sec > 0 => {
                tags.duration_sec.abs_diff(stick_secs) <= LENGTH_TOLERANCE_SECS
            }
            _ => true,
        }
    }

    /// A name as the comparison sees it: Unicode composed (NFC), case
    /// folded, each character FAT cannot hold (and every control
    /// character) as `_`, trailing dots and spaces dropped, as rekordbox
    /// writes a name to a Stick.
    #[must_use]
    pub fn comparable(name: &str) -> String {
        let mut out: String = name
            .nfc()
            .flat_map(char::to_lowercase)
            .map(|c| match c {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                c if (c as u32) < 0x20 => '_',
                c => c,
            })
            .collect();
        while out.ends_with('.') || out.ends_with(' ') {
            out.pop();
        }
        out
    }

    fn stem(name: &str) -> String {
        Path::new(name)
            .file_stem()
            .map_or_else(|| name.to_owned(), |s| s.to_string_lossy().into_owned())
    }

    /// Whether the file's name starts with the whole of the Stick's stem
    /// (rekordbox may cut a name short, never lengthen it), both compared
    /// as [`comparable`] makes them.
    #[must_use]
    pub fn whole_name_agrees(stick: &str, folder: &str) -> bool {
        let (a, b) = (uncut(&comparable(&stem(stick))), comparable(&stem(folder)));
        !a.is_empty() && b.starts_with(&a)
    }

    /// How many characters of a long stem rekordbox keeps when it cuts it.
    pub const CUT_STEM_CHARS: usize = 41;

    /// A Stick stem rekordbox cut: its first [`CUT_STEM_CHARS`] characters
    /// then `-N` (`-1`, `-2`, ...), seen on every cut name of a real Stick.
    /// What is left of the name is those characters; any other stem is as
    /// it is.
    fn uncut(stem: &str) -> String {
        let chars: Vec<char> = stem.chars().collect();
        let cut = chars.len() > CUT_STEM_CHARS + 1
            && chars[CUT_STEM_CHARS] == '-'
            && chars[CUT_STEM_CHARS + 1..].iter().all(char::is_ascii_digit);
        if cut {
            chars[..CUT_STEM_CHARS].iter().collect()
        } else {
            stem.to_owned()
        }
    }

    /// Whether two file names start alike: their stems, compared as
    /// [`comparable`] makes them, agree on the first
    /// [`AGREEING_CHARACTERS`] characters, or on all of the shorter one.
    #[must_use]
    pub fn names_agree(stick: &str, folder: &str) -> bool {
        let (a, b): (Vec<char>, Vec<char>) = (
            comparable(&stem(stick)).chars().collect(),
            comparable(&stem(folder)).chars().collect(),
        );
        let n = AGREEING_CHARACTERS.min(a.len()).min(b.len());
        n > 0 && a[..n] == b[..n]
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
        // A name the Stick's file system spells otherwise (case, Unicode
        // normalisation) is found by listing its folders, as Import
        // Playlist finds it; only names are listed.
        let mut files = rbl_export::device_library::StickFiles::new(root);
        for track in &library.tracks {
            let spelled = root.join(track.path.replace('\\', "/").trim_start_matches('/'));
            let at = if spelled.exists() {
                spelled
            } else {
                files.locate(&track.path)
            };
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
    /// `(parent, name, folder)` -> ids in the tree's order, for the plain
    /// playlists and folders: same-named siblings are each used once.
    nodes: HashMap<(String, String, bool), Vec<String>>,
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
            nodes.entry((parent, name, folder)).or_insert_with(Vec::new).push(id);
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
    /// Test mode only: the step to stop after ([`Rebuild::fail_after`]).
    fail_after: Option<String>,
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

/// What the Library's My Tags become: each Stick tag's place, the
/// categories to rename, and what to report.
type TagPlan = (
    HashMap<u32, TagTarget>,
    Vec<(String, String)>,
    Vec<CategoryOutcome>,
);

/// Maps each Stick category to the Library's category in the same slot
/// (renamed to the Stick's name when the two differ) and each of its tags
/// to the tag of that name there, or one to make.
fn plan_tags(
    stick: &StickState,
    state: &LibraryState,
    matched_files: &HashMap<u32, PathBuf>,
) -> Result<TagPlan, ServiceError> {
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
    Ok((tags, renames, categories))
}

/// The Stick's tree in its order, each node to reuse or make, with the
/// entries carried and those left out.
fn plan_tree(
    stick: &StickState,
    state: &LibraryState,
    matched_files: &HashMap<u32, PathBuf>,
) -> (Vec<PlannedNode>, Vec<PlaylistOutcome>) {
    // The tree, in the Stick's order; a node is reused only under a reused
    // parent (a new folder holds nothing yet).
    let mut library_ids: HashMap<u32, Option<String>> =
        HashMap::from([(0, Some("root".to_owned()))]);
    // Each existing node goes to one Stick node only: same-named siblings
    // are matched one to one, in order.
    let mut unused = state.nodes.clone();
    let mut paths: HashMap<u32, String> = HashMap::new();
    let mut nodes = Vec::new();
    let mut playlists = Vec::new();
    for node in &stick.library.nodes {
        let parent_here = library_ids.get(&node.parent).cloned().flatten();
        let reuse = parent_here.and_then(|parent| {
            let free = unused.get_mut(&(parent, node.name.clone(), node.folder))?;
            (!free.is_empty()).then(|| free.remove(0))
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

    (nodes, playlists)
}

/// The matched tracks planned: each with its values as the Library has
/// them and as the Stick does, and how many tracks each value is carried for.
type TrackPlan = (
    Vec<PlannedTrack>,
    Vec<MatchedTrack>,
    BTreeMap<String, usize>,
);

/// One value of a matched track: `set` for a track the rebuild adds (when
/// the Stick holds one), `kept` when the Library already has the Stick's,
/// `replaced` when the Stick's replaces it.
fn outcome_of(value: &str, to: Value, held: bool, before: Option<Value>) -> Option<ValueOutcome> {
    let (outcome, from) = match before {
        None if held => ("set", None),
        None => return None,
        Some(before) if before == to => ("kept", None),
        Some(before) => ("replaced", Some(before)),
    };
    Some(ValueOutcome {
        value: value.into(),
        outcome: outcome.into(),
        from,
        to,
    })
}

/// Plans each matched track's values (see the module's rules).
fn plan_tracks(
    stick: &StickState,
    state: &LibraryState,
    matched_files: &HashMap<u32, PathBuf>,
    retagged: &std::collections::HashSet<u32>,
    uncertain: &std::collections::HashSet<u32>,
    tags: &HashMap<u32, TagTarget>,
) -> TrackPlan {
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
            if let Some(outcome) = outcome_of(value, stick_value, held, before) {
                outcomes.push(outcome);
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
        // A track with no analysis of its own (new, or one a cut-off
        // apply added) takes the Stick's tempo and key.
        if existing.as_ref().is_none_or(|e| !e.analysed) {
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
            retagged: retagged.contains(&track.id),
            uncertain: uncertain.contains(&track.id),
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

    (planned, matched, values)
}

fn plan(
    request: &Rebuild,
    music: &Path,
    stick: &StickState,
    folder: &matching::MusicFolder,
    library: &OpenLibrary,
) -> Result<Plan, ServiceError> {
    let mut matched_files: HashMap<u32, PathBuf> = HashMap::new();
    let mut retagged = std::collections::HashSet::new();
    let mut uncertain = std::collections::HashSet::new();
    let mut unmatched = Vec::new();
    for track in &stick.library.tracks {
        let shown = |p: &[PathBuf]| p.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let accept = matching::Accept {
            retagged: request.accept_retagged,
            uncertain: request.accept_uncertain,
            check_length: request.check_length,
        };
        let (reason, candidates) = match folder.find(
            &track.path,
            stick.sizes.get(&track.id).copied(),
            track.duration_sec,
            accept,
        ) {
            matching::Found::File(file) => {
                matched_files.insert(track.id, file);
                continue;
            }
            matching::Found::Retagged(file) => {
                retagged.insert(track.id);
                matched_files.insert(track.id, file);
                continue;
            }
            matching::Found::AcceptedUncertain(file) => {
                uncertain.insert(track.id);
                matched_files.insert(track.id, file);
                continue;
            }
            matching::Found::Uncertain(file) => ("uncertain", shown(&[file])),
            matching::Found::NoFile => ("no_file", Vec::new()),
            matching::Found::Ambiguous(files) => ("ambiguous", shown(&files)),
            matching::Found::NameDisagrees(file) => ("name_disagrees", shown(&[file])),
            matching::Found::DifferentSize(file) => ("different_size", shown(&[file])),
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

    let (tags, renames, categories) = plan_tags(stick, &state, &matched_files)?;
    let (planned, matched, values) =
        plan_tracks(stick, &state, &matched_files, &retagged, &uncertain, &tags);
    let (nodes, playlists) = plan_tree(stick, &state, &matched_files);

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
        fail_after: request.fail_after.clone(),
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
            backup: None,
            complete: true,
            incomplete: None,
        },
    })
}

/// The steps of an apply, in order. Each is complete before the next
/// begins; a failure leaves the steps before it done and the one it was in
/// part done, and running the rebuild again finishes it (each step reads
/// what the Library has now, and does only what is still to do).
pub const STEPS: [&str; 8] = [
    "temporary_names",
    "category_names",
    "tags",
    "imports",
    "analysis_files",
    "values",
    "playlists",
    "order",
];

/// The Library's identifiers the apply made, and its Ledger records.
#[derive(Default)]
struct Applied {
    /// Why a track's analysis could not be carried, by its Stick id.
    analysis_failed: HashMap<u32, String>,
    tracks: HashMap<u32, String>,
    nodes: HashMap<u32, String>,
    tags: HashMap<u32, String>,
    records: Vec<Value>,
    /// The steps finished.
    done: Vec<String>,
    /// Why the apply stopped before its end, when it did.
    stopped: Option<String>,
}

/// Analysis files a track was given, and its cues.
struct Carried {
    relative: String,
    cues: Vec<rbl_anlz::CueEntry>,
    /// The files this apply wrote (not those already there, byte for byte).
    files: Vec<PathBuf>,
}

impl Carried {
    /// Takes the files written away again, for an edit that failed after them.
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
/// needed. A crash between the files and the row leaves files no row names:
/// a file already at the place that is byte for byte the one to write (an
/// earlier, cut-off rebuild's) is kept as it is; any other is left alone
/// and the track reported. The Stick is only read, one read per track,
/// below its mount.
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
    for (path, bytes) in &files {
        if path.exists() && std::fs::read(path).ok().as_deref() != Some(bytes.as_slice()) {
            return Err(format!(
                "other analysis files are already at {}; they were left as they are",
                path.display()
            ));
        }
    }
    let mut carried = Carried {
        relative,
        cues: ext
            .as_ref()
            .map_or_else(|| dat.legacy_cue_entries(), rbl_anlz::Anlz::cue_entries),
        files: Vec::new(),
    };
    for (path, bytes) in &files {
        if path.exists() {
            continue;
        }
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

/// The end of a step: recorded as done, and, in test mode only, where a
/// test asked the apply to stop as a crash would ([`Rebuild::fail_after`]).
fn finished(plan: &Plan, applied: &mut Applied, step: &str) -> Result<(), String> {
    applied.done.push(step.to_owned());
    if rbl_db::test_mode() && plan.fail_after.as_deref() == Some(step) {
        return Err(format!("stopped after {step} (test)"));
    }
    Ok(())
}

/// Every write of the rebuild, inside the one edit. A failure part way is
/// not an error of the edit: what was written is kept (each write committed
/// on its own), the Library is re-read and clients told as for a whole
/// apply, and [`Applied::stopped`] says why it stopped.
#[allow(clippy::unnecessary_wraps, reason = "the edit path's closure shape")]
fn apply(writer: &mut rbl_db::write::Writer, plan: &Plan) -> Result<Applied, DbError> {
    let mut applied = Applied::default();
    if let Err(error) = apply_steps(writer, plan, &mut applied) {
        applied.stopped = Some(error);
    }
    Ok(applied)
}

fn apply_steps(
    writer: &mut rbl_db::write::Writer,
    plan: &Plan,
    applied: &mut Applied,
) -> Result<(), String> {
    let db = |e: DbError| e.to_string();
    // Each renamed category under a name no other has first, so that slots
    // trading names never clash; then under the Stick's name. A category
    // left at its temporary name by a cut-off apply is planned again (its
    // name is not the Stick's) and takes the Stick's name now.
    for (category, _) in &plan.renames {
        writer
            .rename_my_tag(category, &format!("rebuild-from-stick {category}"))
            .map_err(db)?;
    }
    finished(plan, applied, "temporary_names")?;
    for (category, name) in &plan.renames {
        writer.rename_my_tag(category, name).map_err(db)?;
        applied.records.push(
            json!({"operation": "rebuild_from_stick", "my_tag_category": category, "name": name}),
        );
    }
    finished(plan, applied, "category_names")?;
    for (stick_id, target) in &plan.tags {
        let id = match target {
            TagTarget::Exists(id) => id.clone(),
            TagTarget::Create { category, name } => {
                // As the window makes one: the stock name, then the rename.
                let made = writer.create_my_tag(category).map_err(db)?;
                writer.rename_my_tag(&made.id, name).map_err(db)?;
                applied.records.push(json!({"operation": "rebuild_from_stick", "my_tag": made.id, "name": name, "category": category}));
                made.id
            }
        };
        applied.tags.insert(*stick_id, id);
    }
    finished(plan, applied, "tags")?;

    // Every new track first, so a cut-off apply leaves tracks a rerun finds
    // at their files and finishes like any track with nothing of its own.
    let mut added = std::collections::HashSet::new();
    for track in &plan.tracks {
        let id = match &track.existing {
            Some(existing) => existing.id.clone(),
            None => {
                if let Some(id) = writer.track_id_at(&track.file).map_err(db)? {
                    id
                } else {
                    let id = writer.import_file(&track.file).map_err(db)?;
                    added.insert(id.clone());
                    id
                }
            }
        };
        applied.tracks.insert(track.stick_id, id);
    }
    finished(plan, applied, "imports")?;

    let mut files_step = false;
    for track in &plan.tracks {
        apply_track(writer, plan, applied, track, &added, &mut files_step)?;
    }
    if !files_step {
        finished(plan, applied, "analysis_files")?;
    }
    finished(plan, applied, "values")?;

    // The tree: each node made or reused, its entries as the Stick lists
    // them, a repeated entry included.
    let mut children: Vec<(String, Vec<String>)> = Vec::new();
    for node in &plan.nodes {
        let parent = if node.stick_parent == 0 {
            "root".to_owned()
        } else {
            applied
                .nodes
                .get(&node.stick_parent)
                .cloned()
                .unwrap_or_else(|| "root".to_owned())
        };
        let id = match &node.reuse {
            Some(id) => id.clone(),
            None if node.folder => writer.create_folder(&node.name, &parent).map_err(db)?,
            None => writer.create_playlist(&node.name, &parent).map_err(db)?,
        };
        if !node.folder {
            let contents: Vec<String> = node
                .entries
                .iter()
                .filter_map(|t| applied.tracks.get(t).cloned())
                .collect();
            set_entries(writer, &id, &contents).map_err(db)?;
        }
        match children.iter_mut().find(|(p, _)| *p == parent) {
            Some((_, ids)) => ids.push(id.clone()),
            None => children.push((parent.clone(), vec![id.clone()])),
        }
        applied.records.push(json!({
            "operation": "rebuild_from_stick",
            "playlist": id,
            "name": node.name,
            "folder": node.folder,
            "made": node.reuse.is_none(),
            "tracks": node.entries.len(),
        }));
        applied.nodes.insert(node.stick_id, id);
    }
    finished(plan, applied, "playlists")?;

    // Each parent's children in the Stick's order, the Library's own after.
    for (parent, ids) in &children {
        put_in_order(writer, parent, ids).map_err(db)?;
    }
    finished(plan, applied, "order")?;
    Ok(())
}

/// One matched track's analysis and values, inside the one edit:
/// `files_step` says whether the first analysis files were written yet
/// (the `analysis_files` step ends there).
fn apply_track(
    writer: &mut rbl_db::write::Writer,
    plan: &Plan,
    applied: &mut Applied,
    track: &PlannedTrack,
    added: &std::collections::HashSet<String>,
    files_step: &mut bool,
) -> Result<(), String> {
    let db = |e: DbError| e.to_string();
    let id = applied.tracks[&track.stick_id].clone();
    let mut changed = Vec::new();
    let carried = match &track.analysis {
        Some(dat) => match carry_analysis(writer, plan, dat, &track.file, &id) {
            Ok(carried) => Some(carried),
            Err(reason) => {
                changed.push(json!({"field": "analysis", "failed": reason}));
                applied.analysis_failed.insert(track.stick_id, reason);
                None
            }
        },
        None => None,
    };
    if carried.is_some() && !*files_step {
        // The first files written, their row not yet.
        *files_step = true;
        finished(plan, applied, "analysis_files")?;
    }
    // The Stick's tempo and key go with its grid, or to a track with no
    // analysis of its own (one just added, or one a cut-off apply added).
    let own = track.existing.as_ref().is_some_and(|e| e.analysed);
    if !own || carried.is_some() {
        if !track.key.is_empty() {
            writer.ensure_detected_key(&track.key).map_err(db)?;
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
            return Err(error.to_string());
        }
        if let Some(carried) = &carried {
            changed.push(
                json!({"field": "analysis", "path": carried.relative, "cues": carried.cues.len()}),
            );
        }
    }
    let before = track.existing.as_ref();
    if before.map_or(track.rating > 0, |e| e.rating != track.rating) {
        writer.set_rating(&id, track.rating).map_err(db)?;
        changed
            .push(json!({"field": "rating", "from": before.map(|e| e.rating), "to": track.rating}));
    }
    if before.map_or(track.colour > 0, |e| e.colour != track.colour) {
        let colour = (track.colour > 0).then(|| track.colour.to_string());
        writer.set_color(&id, colour.as_deref()).map_err(db)?;
        changed
            .push(json!({"field": "colour", "from": before.map(|e| e.colour), "to": track.colour}));
    }
    // A new track's comment is the Stick's, empty included: the file's
    // own comment, which the import read, is not the DJ's.
    if before.is_none_or(|e| e.comment != track.comment) {
        writer.set_comment(&id, &track.comment).map_err(db)?;
        changed.push(json!({"field": "comment", "from": before.map(|e| e.comment.clone()), "to": track.comment}));
    }
    if !track.tags.is_empty() {
        let mut all: Vec<String> = before.map(|e| e.tags.clone()).unwrap_or_default();
        let start = all.len();
        for stick_tag in &track.tags {
            let tag = &applied.tags[stick_tag];
            if !all.contains(tag) {
                all.push(tag.clone());
            }
        }
        if all.len() > start {
            writer.set_my_tags(&id, &all).map_err(db)?;
            changed.push(json!({"field": "my_tags", "added": &all[start..]}));
        }
    }
    applied.records.push(json!({
        "operation": "rebuild_from_stick",
        "track": id,
        "file": track.file,
        "added": added.contains(&id),
        "changed": changed,
    }));
    Ok(())
}

/// A playlist's entries in order, each repeat its own entry.
fn entries_of(writer: &rbl_db::write::Writer, playlist: &str) -> Result<Vec<String>, DbError> {
    let conn = writer.library().connection();
    let mut stmt = conn.prepare(
        "SELECT ContentID FROM djmdSongPlaylist WHERE PlaylistID = ?1 AND rb_local_deleted = 0 ORDER BY TrackNo",
    )?;
    let entries = stmt
        .query_map([playlist], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(entries)
}

/// Makes `playlist` hold `contents` in that order, a repeated track as
/// often as it is listed (`set_tracks` keeps only the first of a repeat):
/// emptied, then filled with repeats allowed. Nothing is written when it
/// holds exactly these already. A cut between the two leaves it empty, and
/// a rerun fills it.
fn set_entries(
    writer: &mut rbl_db::write::Writer,
    playlist: &str,
    contents: &[String],
) -> Result<(), DbError> {
    if entries_of(writer, playlist)? == contents {
        return Ok(());
    }
    writer.set_tracks(playlist, &[])?;
    if !contents.is_empty() {
        writer.add_tracks_allowing(playlist, contents, true)?;
    }
    Ok(())
}

/// Puts the children `ids` of `parent` first, in that order, the parent's
/// other children after them as they were. Nothing is written when they
/// are already so.
fn put_in_order(
    writer: &mut rbl_db::write::Writer,
    parent: &str,
    ids: &[String],
) -> Result<(), DbError> {
    let now: Vec<String> = {
        let conn = writer.library().connection();
        let mut stmt = conn.prepare(
            "SELECT ID FROM djmdPlaylist
             WHERE COALESCE(NULLIF(ParentID, ''), 'root') = ?1 AND rb_local_deleted = 0 ORDER BY Seq, ID",
        )?;
        let rows = stmt
            .query_map([parent], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    if now.len() >= ids.len() && now[..ids.len()] == *ids {
        return Ok(());
    }
    for (place, id) in ids.iter().enumerate() {
        writer.move_to(id, parent, Some(place))?;
    }
    Ok(())
}

/// Answers `library.rebuild_from_stick`, and what to tell the hosting
/// window when the Library changed.
pub(crate) fn answer(
    library: &OpenLibrary,
    stems: &crate::stems::Stems,
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
    // A backup first, so a result the DJ does not want is put back whole;
    // no backup, no rebuild.
    let backup = crate::large_run::before_write_backed_up(library, stems)?.ok_or_else(|| {
        ServiceError::new(
            shared_codes::WRITE_REFUSED,
            "a backup of the Library could not be taken, so nothing was changed",
        )
    })?;
    let shown_backup = backup.to_string_lossy().into_owned();
    // One import Run under the caller's source, one edit.
    let operation = library.import_operation();
    let capacity = 1 + plan.tags.len() + plan.tracks.len() + plan.nodes.len() + plan.renames.len();
    let result = operation.run_many(
        json!({"operation": "rebuild_from_stick", "stick": request.stick, "music": music}),
        capacity,
        || {
            // The backup is still of the moment before the write; the gate
            // is held through it (reentrant), and let go before recording.
            let _gate = crate::large_run::bound(library, stems, &[])?;
            library.edit(Touched::Tracks, |writer| apply(writer, &plan))
        },
        |(applied, _)| {
            let mut records =
                vec![json!({"operation": "rebuild_from_stick", "backup": shown_backup})];
            records.extend(applied.records.iter().cloned());
            records
        },
    );
    let warning = operation.finish();
    let (applied, changed) = result?;
    plan.answer.backup = Some(shown_backup.clone());
    if let Some(error) = &applied.stopped {
        plan.answer.complete = false;
        plan.answer.incomplete = Some(Incomplete {
            error: error.clone(),
            done: applied.done.clone(),
            advice: format!(
                "The rebuild stopped part way; what it wrote is kept. Nothing is lost: run again to finish it \
                 (it does only what is still to do), or restore the backup taken before it, {shown_backup} \
                 (wreckordbox backup restore), to put the Library back as it was."
            ),
        });
    }
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
#[allow(clippy::unwrap_used, reason = "test fixtures")]
mod tests {
    use super::matching::{names_agree, whole_name_agrees, Accept, Found, MusicFolder};

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

    const PLAIN: Accept = Accept {
        retagged: false,
        uncertain: false,
        check_length: true,
    };
    const RETAGGED: Accept = Accept {
        retagged: true,
        uncertain: false,
        check_length: true,
    };
    const UNCERTAIN: Accept = Accept {
        retagged: false,
        uncertain: true,
        check_length: true,
    };

    #[test]
    fn a_file_is_found_by_its_size_and_its_whole_name_and_never_guessed() {
        let (dir, music) = folder(&[
            ("x/One (Extended Mix).mp3", 10),
            ("Two.mp3", 5),
            ("a/Two.MP3", 6),
            ("b/Three.mp3", 7),
            ("c/three copy.mp3", 7),
            ("Stranger.mp3", 8),
            ("._Stranger.mp3", 9),
            ("What? Now.mp3", 12),
            ("notes.txt", 1),
        ]);
        let at = |p: &str| dir.path().join(p);
        assert_eq!(music.files, 7, "AppleDouble and non-audio files skipped");
        // A name the Stick cut short: the size, the whole of its name agreeing.
        assert_eq!(
            music.find("/Contents/A/B/One (Extended Mi.mp3", Some(10), 0, PLAIN),
            Found::File(at("x/One (Extended Mix).mp3"))
        );
        assert_eq!(
            music.find("/Contents/Two.mp3", Some(6), 0, PLAIN),
            Found::File(at("a/Two.MP3"))
        );
        // A character FAT cannot hold, as rekordbox writes it.
        assert_eq!(
            music.find("/Contents/What_ Now.mp3", Some(12), 0, PLAIN),
            Found::File(at("What? Now.mp3"))
        );
        // Several of that size, or one whose name does not agree.
        assert_eq!(
            music.find("/Contents/Three.mp3", Some(7), 0, PLAIN),
            Found::Ambiguous(vec![at("b/Three.mp3"), at("c/three copy.mp3")])
        );
        assert_eq!(
            music.find("/Contents/Gone.mp3", Some(8), 0, PLAIN),
            Found::NameDisagrees(at("Stranger.mp3"))
        );
        // No file of that size: the same name, retagged.
        assert_eq!(
            music.find("/Contents/One (Extended Mix).mp3", Some(11), 0, PLAIN),
            Found::DifferentSize(at("x/One (Extended Mix).mp3"))
        );
        assert_eq!(
            music.find("/Contents/One (Extended Mix).mp3", Some(11), 0, RETAGGED),
            Found::Retagged(at("x/One (Extended Mix).mp3"))
        );
        assert_eq!(
            music.find("/Contents/Two.mp3", Some(9), 0, RETAGGED),
            Found::Ambiguous(vec![at("Two.mp3"), at("a/Two.MP3")])
        );
        assert_eq!(
            music.find("/Contents/._Stranger.mp3", Some(9), 0, RETAGGED),
            Found::NoFile
        );
        assert_eq!(
            music.find("/Contents/Four.mp3", Some(1), 0, RETAGGED),
            Found::NoFile
        );
        // No Stick copy to measure: a lone file of the name.
        assert_eq!(
            music.find("/Contents/stranger.MP3", None, 0, PLAIN),
            Found::File(at("Stranger.mp3"))
        );
        assert_eq!(
            music.find("/Contents/Two.mp3", None, 0, PLAIN),
            Found::Ambiguous(vec![at("Two.mp3"), at("a/Two.MP3")])
        );
    }

    #[test]
    fn another_mix_of_the_same_size_is_uncertain_and_taken_only_when_accepted() {
        // The Stick's mix is missing; another of exactly its size is there,
        // its name agreeing over twenty characters but not over the whole.
        let (dir, music) = folder(&[("Night Drive Anthem (Club Mix).mp3", 40)]);
        let wrong = dir.path().join("Night Drive Anthem (Club Mix).mp3");
        let stick = "/Contents/X/Y/Night Drive Anthem (Extended Mix).mp3";
        assert_eq!(
            music.find(stick, Some(40), 0, PLAIN),
            Found::Uncertain(wrong.clone())
        );
        assert_eq!(
            music.find(stick, Some(40), 0, UNCERTAIN),
            Found::AcceptedUncertain(wrong)
        );
    }

    /// A silent 8 kHz mono WAV of `secs` seconds.
    fn wav(path: &std::path::Path, secs: u32) {
        let data_len = 8_000 * 2 * secs;
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16_u32.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(&8_000_u32.to_le_bytes());
        out.extend_from_slice(&16_000_u32.to_le_bytes());
        out.extend_from_slice(&2_u16.to_le_bytes());
        out.extend_from_slice(&16_u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        out.resize(44 + data_len as usize, 0);
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn a_length_that_does_not_agree_makes_a_size_match_uncertain() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Song.wav");
        wav(&file, 3);
        let size = std::fs::metadata(&file).unwrap().len();
        let music = MusicFolder::read(dir.path()).unwrap();
        assert_eq!(
            music.find("/Contents/Song.wav", Some(size), 3, PLAIN),
            Found::File(file.clone())
        );
        assert_eq!(
            music.find("/Contents/Song.wav", Some(size), 4, PLAIN),
            Found::File(file.clone()),
            "within a second"
        );
        assert_eq!(
            music.find("/Contents/Song.wav", Some(size), 200, PLAIN),
            Found::Uncertain(file.clone())
        );
        let names_only = Accept {
            check_length: false,
            ..PLAIN
        };
        assert_eq!(
            music.find("/Contents/Song.wav", Some(size), 200, names_only),
            Found::File(file)
        );
    }

    #[test]
    fn the_whole_of_the_sticks_name_must_agree_for_a_certain_match() {
        assert!(whole_name_agrees(
            "Short (Original Mi.wav",
            "Short (Original Mix).wav"
        ));
        assert!(whole_name_agrees("CAPS.wav", "caps.aiff"));
        assert!(
            whole_name_agrees("Cafe\u{301}.wav", "Caf\u{e9}.wav"),
            "composed and decomposed"
        );
        assert!(!whole_name_agrees(
            "Night Drive Anthem (Extended Mix).wav",
            "Night Drive Anthem (Club Mix).wav"
        ));
        assert!(names_agree(
            "Night Drive Anthem (Extended Mix).wav",
            "Night Drive Anthem (Club Mix).wav"
        ));
        assert!(!names_agree("Gone.wav", "Stranger.wav"));
    }

    #[test]
    fn a_name_rekordbox_cut_to_forty_one_characters_and_numbered_agrees() {
        // rekordbox cuts a long stem to its first 41 characters and adds
        // `-N`: the whole of what is left of the name is those 41.
        let folder = "Abcdefghij Klmnopqrst Uvwxyzabcd (Extended Original Mix).aiff";
        let cut: String = folder.chars().take(41).collect();
        assert!(whole_name_agrees(&format!("{cut}-1.aiff"), folder));
        assert!(whole_name_agrees(&format!("{cut}-12.aiff"), folder));
        // Not the 41 characters: a number that is part of the name.
        assert!(!whole_name_agrees(
            "Abcdefghij Klmnopqrst Uvwxyz-1.aiff",
            folder
        ));
        let other: String =
            "Abcdefghij Klmnopqrst Uvwxyzabcd (Radio Edit and more words).aiff".to_owned();
        assert!(!whole_name_agrees(&format!("{cut}-1.aiff"), &other));
    }
}
