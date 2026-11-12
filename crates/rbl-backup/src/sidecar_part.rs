//! The Sidecar's part of a backup: what wreckordbox knows beside the
//! Library (Facts, Runs, Stems, Mutants, the kinds file, the Tag schema and
//! Recipes), so that a restored Library comes back with it.
//!
//! The Sidecar is the folder [`dir`] names, beside `master.db` unless
//! [`DIR_ENV`] says otherwise. What a backup holds of it is decided by rule,
//! not by a list of files, so a file a later version adds travels without a
//! change here:
//!
//! - Everything in the folder is backed up, under `sidecar/…` in the
//!   archive, except the entries below.
//! - Every SQLite database in it is taken as a snapshot (`VACUUM INTO`), so
//!   it is whole even while the Library service is writing; its `-wal`,
//!   `-shm` and `-journal` companions are not copied.
//! - The top-level folders in [`REGENERABLE`] are never backed up. They are
//!   reserved for large data that can be made again: a tool's work area,
//!   audition files, render caches, any other cache.
//! - Stem audio, the top-level [`STEMS_FOLDER`], is left out unless
//!   [`Options::stems`] asks for it. It is about 215 MB a track and takes
//!   over twenty minutes a track to make again.
//! - Top-level entries whose names start with a dot are left out.
//!
//! `sidecar.json` in the archive, the [`Contents`], says the part is there
//! and whether Stem audio is in it. A backup without that entry (every
//! backup made before this module, and one taken when no Sidecar existed)
//! restores as before and leaves the Sidecar on disk alone.
//!
//! A restore puts the Sidecar back with the database, never apart from it,
//! since Facts and Mutants are about that database's tracks and playlists.
//! Each top-level entry is replaced through the restore journal like any
//! other target; a backed-up kind of entry that is on disk but not in the
//! backup is set aside, so the folder ends as it was when the backup was
//! taken. What a backup never holds (the reserved folders, and Stem audio
//! when it was left out) is not touched.
use crate::{archive::Archive, refused, Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// The Sidecar's folder name beside `master.db`. The same as
/// `wreckordbox_sidecar::FOLDER_NAME`; this crate does not depend on that
/// one, and a test holds the two together.
pub const FOLDER_NAME: &str = "wreckordbox-sidecar";

/// Names another folder for the Sidecar, as the Library service reads it
/// (`wreckordbox_service::stems::SIDECAR_DIR_ENV`).
pub const DIR_ENV: &str = "WRECKORDBOX_SIDECAR_DIR";

/// The archive entry that says a backup holds a Sidecar.
pub const NAME: &str = "sidecar.json";

/// The archive folder the Sidecar's entries are under.
pub const ARCHIVE_FOLDER: &str = "sidecar";

/// The version of [`Contents`] this crate writes and the newest it restores.
pub const VERSION: u32 = 1;

/// Stem audio: `stems/<track>/run-<run>/<role>.wav`.
pub const STEMS_FOLDER: &str = "stems";

/// Top-level folders of the Sidecar that are never backed up and never
/// touched by a restore, reserved for data that can be made again:
///
/// - `work`: a tool's output before it is accepted (emptied whenever the
///   Sidecar is opened).
/// - `auditions`: audition files of Stem edits.
/// - `renders`: the cache of rendered Mutants.
/// - `cache`: anything else derived.
pub const REGENERABLE: [&str; 4] = ["work", "auditions", "renders", "cache"];

/// Where the choices in [`Options`] are saved, in [`crate::state_dir`].
pub const OPTIONS_FILE: &str = "backup-sidecar.json";

/// Larger than any real [`Contents`].
const MAX_BYTES: u64 = 1024 * 1024;

/// The Sidecar of the Library at `location`: [`DIR_ENV`]'s folder, or
/// [`FOLDER_NAME`] beside `master.db`.
pub fn dir(location: &rbl_db::LibraryLocation) -> PathBuf {
    dir_from(&location.master_db, std::env::var_os(DIR_ENV))
}

/// As [`dir`], with the environment's choice given.
pub fn dir_from(master_db: &Path, chosen: Option<OsString>) -> PathBuf {
    match chosen.filter(|chosen| !chosen.is_empty()) {
        Some(chosen) => PathBuf::from(chosen),
        None => master_db.parent().map_or_else(|| PathBuf::from(FOLDER_NAME), |root| root.join(FOLDER_NAME)),
    }
}

/// The Sidecar's folder, checked before a backup reads it or a restore
/// writes in it, and pinned: every later step uses this path, and
/// [`Destination::verify`] checks it is still the same folder just before
/// anything is replaced. A restore's journal records it, so recovery acts
/// on the folder the restore checked, whatever the environment of the
/// program that recovers.
///
/// Refused: a relative path; a folder that is a symbolic link (the writes
/// would land wherever it points, a Stick included); something that is
/// not a folder; and a folder that holds the Library's folder, or overlaps
/// `share/` or anything a restore replaces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    /// The folder, with the folders above it resolved.
    pub path: PathBuf,
}

impl Destination {
    /// The Sidecar of the Library at `location`, as [`dir`] names it, checked.
    pub fn of(location: &rbl_db::LibraryLocation) -> Result<Self> {
        Self::check(location, &dir(location))
    }

    /// `path` as the Sidecar of the Library at `location`, checked.
    pub fn check(location: &rbl_db::LibraryLocation, path: &Path) -> Result<Self> {
        let shown = path.display();
        if !path.is_absolute() || path.components().any(|part| !matches!(part, std::path::Component::Normal(_) | std::path::Component::RootDir | std::path::Component::Prefix(_))) {
            return Err(refused(format!("The Sidecar's folder must be a plain absolute path: {shown}")));
        }
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(refused(format!("The Sidecar's folder cannot be {shown}.")));
        };
        own_folder(path)?;
        let pinned = resolved(parent)?.join(name);
        let library_root = location.master_db.parent().map(resolved).transpose()?;
        let mut replaced: Vec<PathBuf> = crate::journal::restorable(location).iter().map(|path| resolved(path)).collect::<Result<_>>()?;
        replaced.push(resolved(&location.share_root)?);
        let holds_library = library_root.as_ref().is_some_and(|root| root.starts_with(&pinned));
        if holds_library || replaced.iter().any(|other| other.starts_with(&pinned) || pinned.starts_with(other)) {
            return Err(refused(format!(
                "The Sidecar's folder {shown} overlaps the Library's own files. Choose a folder of its own for the Sidecar."
            )));
        }
        Ok(Self { path: pinned })
    }

    /// Whether the folder is still the one that was checked: not replaced
    /// by a symbolic link or a file, and not moved under a link since.
    pub fn verify(&self) -> Result<()> {
        own_folder(&self.path)?;
        let parent = self.path.parent().ok_or_else(|| refused("Invalid Sidecar folder."))?;
        if resolved(parent)? != parent {
            return Err(refused(format!("The Sidecar's folder {} moved while it was being restored.", self.path.display())));
        }
        Ok(())
    }

    /// Whether a restore journal may name `target`: only a top-level entry
    /// of this folder, of a kind a backup can hold.
    pub(crate) fn may_replace(&self, target: &Path) -> bool {
        target.parent() == Some(self.path.as_path()) && target.file_name().and_then(|name| name.to_str()).is_some_and(|name| backed_up(name, true))
    }
}

/// The Sidecar's folder itself must be a real folder, or not there yet.
fn own_folder(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(refused(format!(
            "The Sidecar's folder {} is a symbolic link. Backups and restores do not follow it; point {DIR_ENV} at the folder itself.",
            path.display()
        ))),
        Ok(meta) if !meta.is_dir() => Err(refused(format!("The Sidecar's folder {} is not a folder.", path.display()))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// `path` with its nearest existing ancestor resolved and the rest, which
/// does not exist yet, kept as written.
fn resolved(path: &Path) -> Result<PathBuf> {
    let mut missing = Vec::new();
    let mut existing = path;
    loop {
        match existing.canonicalize() {
            Ok(found) => return Ok(missing.iter().rev().fold(found, |path, name| path.join(name))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else { return Err(e.into()) };
                missing.push(name.to_os_string());
                existing = parent;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// What a person chooses about the Sidecar's part of a backup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    /// Include Stem audio. Off unless asked for: about 215 MB a track.
    #[serde(default)]
    pub stems: bool,
}

impl Options {
    /// The choices saved in `state_dir` ([`OPTIONS_FILE`], for example
    /// `{"stems": true}`), or the defaults when none were.
    pub fn saved(state_dir: &Path) -> Self {
        fs::read(state_dir.join(OPTIONS_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}

/// `sidecar.json`: the Sidecar a backup holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contents {
    pub version: u32,
    /// Logical bytes of the entries under `sidecar/`, before compression.
    pub bytes: u64,
    /// Whether Stem audio is in the backup.
    pub stems: bool,
    /// Bytes of Stem audio the Sidecar held when the backup was taken,
    /// whether or not it is in the backup.
    #[serde(default)]
    pub stems_bytes: u64,
    /// Every SQLite database in the part, by its path in the Sidecar
    /// (`sidecar.db`, `recipes/x.db`). A restore checks each of these is
    /// there and sound, however its first bytes read. Empty in backups
    /// made before it was recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub databases: Vec<String>,
}

impl Contents {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let contents: Self = serde_json::from_slice(bytes).map_err(|_| refused("The backup's Sidecar record cannot be read."))?;
        if contents.version == 0 || contents.version > VERSION {
            return Err(refused("This backup's Sidecar was saved by a newer version of wreckordbox. Update wreckordbox to restore it."));
        }
        Ok(contents)
    }

    /// The Sidecar record of a backup ZIP, or `None` when it holds no Sidecar.
    pub fn read(path: &Path) -> Result<Option<Self>> {
        Self::read_from(&mut crate::archive::open(path)?)
    }

    /// As [`Contents::read`], from an archive already open.
    pub fn read_from(zip: &mut Archive) -> Result<Option<Self>> {
        crate::archive::read_small(zip, NAME, MAX_BYTES)?.map(|bytes| Self::parse(&bytes)).transpose()
    }
}

/// Whether a top-level entry of the Sidecar with this name belongs in a
/// backup, with Stem audio (`stems`) or without. The one rule both a backup
/// and a restore go by.
pub fn backed_up(name: &str, stems: bool) -> bool {
    !(name.is_empty() || name.starts_with('.') || REGENERABLE.contains(&name) || (name == STEMS_FOLDER && !stems))
}

/// One file for the archive.
#[derive(Debug)]
pub struct Item {
    /// Its entry name: `sidecar/kinds.toml`, or [`NAME`].
    pub name: String,
    /// The file to read it from: the Sidecar's own, or a database snapshot.
    pub source: PathBuf,
    pub bytes: u64,
}

/// The Sidecar's part of a backup, ready to be written into an archive.
#[derive(Debug)]
pub struct Collected {
    pub contents: Contents,
    /// Folder entries, parents before children: `sidecar`, `sidecar/recipes`.
    pub directories: Vec<String>,
    /// Every file entry, [`NAME`] last. Sources stay readable for as long
    /// as this value lives.
    pub files: Vec<Item>,
    /// Database snapshots and the record.
    _stage: tempfile::TempDir,
}

/// Collects what a backup holds of the Sidecar at `sidecar_dir`: `None`
/// when there is no such folder. `proceed` is asked as the folder is read;
/// `false` stops with [`Error::Cancelled`].
///
/// A symbolic link in what would be backed up is refused, as in the rest of
/// a backup, and so is a database that cannot be snapshotted or fails
/// SQLite's check: a backup that silently lacked them would not be one.
///
/// The folder itself must not be a symbolic link; use [`collect_for`] to
/// have it checked against the Library as well.
pub fn collect(sidecar_dir: &Path, options: Options, proceed: &mut dyn FnMut() -> bool) -> Result<Option<Collected>> {
    own_folder(sidecar_dir)?;
    if !sidecar_dir.is_dir() {
        return Ok(None);
    }
    let stage = tempfile::tempdir()?;
    let mut walk = Walk { options, proceed, stage: stage.path(), snapshots: 0, directories: vec![ARCHIVE_FOLDER.to_owned()], files: Vec::new(), databases: Vec::new() };
    walk.folder(sidecar_dir, ARCHIVE_FOLDER, true)?;
    let Walk { directories, mut files, databases, .. } = walk;
    let contents = Contents {
        version: VERSION,
        bytes: files.iter().map(|item| item.bytes).sum(),
        stems: options.stems,
        stems_bytes: tree_bytes(&sidecar_dir.join(STEMS_FOLDER)),
        databases,
    };
    let record = stage.path().join(NAME);
    let bytes = serde_json::to_vec(&contents)?;
    fs::write(&record, &bytes)?;
    files.push(Item { name: NAME.to_owned(), source: record, bytes: bytes.len() as u64 });
    Ok(Some(Collected { contents, directories, files, _stage: stage }))
}

/// [`collect`] for the Sidecar of the Library at `location`, its folder
/// checked first ([`Destination::of`]).
pub fn collect_for(location: &rbl_db::LibraryLocation, options: Options, proceed: &mut dyn FnMut() -> bool) -> Result<Option<Collected>> {
    collect(&Destination::of(location)?.path, options, proceed)
}

struct Walk<'a> {
    options: Options,
    proceed: &'a mut dyn FnMut() -> bool,
    stage: &'a Path,
    snapshots: usize,
    directories: Vec<String>,
    files: Vec<Item>,
    /// Paths in the Sidecar of the databases snapshotted.
    databases: Vec<String>,
}

impl Walk<'_> {
    fn folder(&mut self, directory: &Path, archived: &str, top: bool) -> Result<()> {
        let mut paths = fs::read_dir(directory)?.map(|entry| entry.map(|entry| entry.path())).collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            if !(self.proceed)() {
                return Err(Error::Cancelled);
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                return Err(refused(format!("The Sidecar holds a file whose name a backup cannot carry: {}", path.display())));
            };
            if top && !backed_up(name, self.options.stems) {
                continue;
            }
            let meta = fs::symlink_metadata(&path)?;
            let entry = format!("{archived}/{name}");
            if meta.file_type().is_symlink() {
                return Err(refused(format!("Symbolic links are not supported in backups: {}", path.display())));
            } else if meta.is_dir() {
                self.directories.push(entry.clone());
                self.folder(&path, &entry, false)?;
            } else if meta.is_file() {
                let store = top && name == DATABASE_FILE;
                if !store && database_companion(&path, top) {
                    continue;
                }
                // The Sidecar's own database is one whatever its first
                // bytes say: a damaged one fails the backup, never travels
                // as a plain file.
                if store || is_database(&path) {
                    let source = self.snapshot(&path, store)?;
                    let bytes = fs::metadata(&source)?.len();
                    self.databases.push(entry.strip_prefix(&format!("{ARCHIVE_FOLDER}/")).unwrap_or(&entry).to_owned());
                    self.files.push(Item { name: entry, source, bytes });
                } else {
                    self.files.push(Item { name: entry, source: path, bytes: meta.len() });
                }
            }
        }
        Ok(())
    }

    /// A whole copy of a database that may be in use, checked.
    fn snapshot(&mut self, database: &Path, store: bool) -> Result<PathBuf> {
        self.snapshots += 1;
        let copy = self.stage.join(format!("database-{}", self.snapshots));
        let shown = database.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let failed = |e: rusqlite::Error| refused(format!("The Sidecar's database {shown} could not be copied into the backup: {e}"));
        let source = rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(failed)?;
        source.execute("VACUUM main INTO ?1", [copy.to_string_lossy().as_ref()]).map_err(failed)?;
        drop(source);
        if !sound(&copy)? {
            return Err(refused(format!("The backup copy of the Sidecar's database {shown} failed its integrity check.")));
        }
        if store && !sidecar_store(&copy)? {
            return Err(refused(format!("The Sidecar's database {shown} is not a Sidecar store; it may be damaged. Nothing was backed up.")));
        }
        Ok(copy)
    }
}

/// The Sidecar's own database, by name, at the top of its folder.
pub const DATABASE_FILE: &str = "sidecar.db";

/// Whether the database at `path` is a Sidecar store: SQLite, sound, and
/// with the schema version the Sidecar writes when it makes one. An empty
/// file is a valid SQLite database, but not a store. Leaves no companion
/// file beside it.
pub(crate) fn sidecar_store(path: &Path) -> Result<bool> {
    if !is_database(path) || !sound(path)? {
        return Ok(false);
    }
    let version = (|| -> rusqlite::Result<Option<String>> {
        use rusqlite::OptionalExtension as _;
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |row| row.get(0))
            .optional()
    })();
    for suffix in COMPANIONS {
        crate::journal::remove(&crate::sidecar(path, suffix))?;
    }
    Ok(version.ok().flatten().and_then(|version| version.parse::<u32>().ok()).is_some_and(|version| version >= 1))
}

/// Whether SQLite's own check passes on the database at `path`. Leaves no
/// companion file beside it.
pub(crate) fn sound(path: &Path) -> Result<bool> {
    let check = (|| -> rusqlite::Result<String> {
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?.query_row("PRAGMA quick_check", [], |row| row.get(0))
    })();
    for suffix in COMPANIONS {
        crate::journal::remove(&crate::sidecar(path, suffix))?;
    }
    Ok(check.is_ok_and(|check| check == "ok"))
}

const COMPANIONS: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Whether the file starts as every SQLite database does.
pub(crate) fn is_database(path: &Path) -> bool {
    let mut head = [0; 16];
    fs::File::open(path).and_then(|mut file| file.read_exact(&mut head)).is_ok() && head == *b"SQLite format 3\0"
}

/// A database's write-ahead log, shared memory or rollback journal. The
/// Sidecar's own database's always are, whatever state it is in.
fn database_companion(path: &Path, top: bool) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return false };
    COMPANIONS.iter().any(|suffix| {
        name.strip_suffix(suffix)
            .is_some_and(|base| !base.is_empty() && ((top && base == DATABASE_FILE) || is_database(&path.with_file_name(base))))
    })
}

/// Bytes of the regular files under `path`; nothing for what cannot be read.
fn tree_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    fs::read_dir(path).map_or(0, |entries| entries.flatten().map(|entry| tree_bytes(&entry.path())).sum())
}

/// The path inside the Sidecar that an archive entry restores: `None` for
/// an entry that is not under `sidecar/`, and an empty path for the folder
/// itself. The name has already passed [`crate::archive::classify`], which
/// refuses any that could leave the folder it is unpacked into.
pub fn entry(name: &str) -> Option<PathBuf> {
    let mut parts = Path::new(name).components();
    (parts.next()?.as_os_str() == ARCHIVE_FOLDER).then(|| parts.collect())
}

/// A friendly refusal ahead of unpacking, on the Sidecar's own volume,
/// which [`DIR_ENV`] can put elsewhere than the Library's.
pub(crate) fn check_space(sidecar_dir: &Path, contents: &Contents) -> Result<()> {
    let Some(existing) = sidecar_dir.ancestors().find(|path| path.exists()) else { return Ok(()) };
    let Ok(free) = fs2::available_space(existing) else { return Ok(()) };
    let needed = contents.bytes.saturating_add(64 * 1024 * 1024);
    if needed > free {
        return Err(refused(format!(
            "There is not enough free disk space to restore the Sidecar. It needs {needed} bytes, and {free} are free."
        )));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_sidecar_is_beside_the_library_unless_the_environment_names_another_folder() {
        let master = Path::new("/lib/rekordbox/master.db");
        assert_eq!(dir_from(master, None), Path::new("/lib/rekordbox/wreckordbox-sidecar"));
        assert_eq!(dir_from(master, Some(OsString::new())), Path::new("/lib/rekordbox/wreckordbox-sidecar"));
        assert_eq!(dir_from(master, Some("/elsewhere/sidecar".into())), Path::new("/elsewhere/sidecar"));
    }

    #[test]
    fn regenerable_folders_hidden_entries_and_unasked_stems_are_not_backed_up() {
        for name in ["sidecar.db", "kinds.toml", "tag-schema.toml", "recipes", "something-new"] {
            assert!(backed_up(name, false), "{name}");
        }
        for name in ["work", "auditions", "renders", "cache", ".git", ".rbxport-restore-1-new", ""] {
            assert!(!backed_up(name, true), "{name}");
        }
        assert!(!backed_up("stems", false));
        assert!(backed_up("stems", true));
    }

    #[test]
    fn a_record_from_a_newer_version_or_not_a_record_is_refused() {
        assert_eq!(Contents::parse(br#"{"version":1,"bytes":7,"stems":false}"#).unwrap(), Contents { version: 1, bytes: 7, stems: false, stems_bytes: 0, databases: Vec::new() });
        assert!(Contents::parse(br#"{"version":2,"bytes":7,"stems":false}"#).is_err());
        assert!(Contents::parse(b"[]").is_err());
    }

    #[test]
    fn sidecar_entries_are_those_under_the_sidecar_folder() {
        assert_eq!(entry("sidecar/kinds.toml"), Some(PathBuf::from("kinds.toml")));
        assert_eq!(entry("sidecar/recipes/friday.toml"), Some(PathBuf::from("recipes/friday.toml")));
        assert_eq!(entry("sidecar/"), Some(PathBuf::new()));
        assert_eq!(entry("sidecar.json"), None);
        assert_eq!(entry("analysis/sidecar/x"), None);
    }
}
