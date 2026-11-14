//! Writing a backup archive from outside the window (wbx-25: the backup a
//! large Run takes first), in two steps.
//!
//! 1. [`stage`]: everything the backup holds is copied to local staging,
//!    where nothing else writes: the Sidecar (its databases snapshotted,
//!    its other files copied), a vacuumed snapshot of `master.db`, the
//!    library files beside it, and the analysis and artwork trees (each
//!    file hard-linked where the file system allows, else copied). The
//!    caller holds the Library's edit gate and the analysis-file lock for
//!    this step only, so both stores and the trees are of one moment: no
//!    edit through the edit path comes between the Sidecar and the Library.
//!    Hard links are an immutable copy because the Library's own writers
//!    publish analysis files and Stem audio by writing a new file and
//!    renaming it over the old (`rbl_core::durable::write`); a writer that
//!    rewrote such a file in place would change the staged copy too.
//! 2. [`publish`]: from the staged copies only, with no lock of the
//!    Library's held, the archive is written to the backup folder (which may
//!    be on a Stick: every write asks `check` first, through [`Checked`]),
//!    under a `.partial-…zip` name, and renamed into place complete, never
//!    replacing a file. On a failure the partial file is removed when
//!    `check` still allows a write; otherwise it is left and named.
//!
//! The archive is the one the window's Preferences › Backups writes, read
//! by the same restore (`wreckordbox backup restore`) and lists: manifest,
//! summary, `master.db`, library files, `analysis/`, `artwork/`, and the
//! Sidecar by [`crate::sidecar_part`]'s rule; named `rbexport-…zip`. It does
//! not carry the window's restore scripts (they are the app shell's).

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::{refused, Error, Result};

/// What to stage.
pub struct Stage<'a> {
    pub location: &'a rbl_db::LibraryLocation,
    /// The Sidecar's folder (checked against the Library), or `None` for a
    /// Library with no Sidecar.
    pub sidecar_dir: Option<&'a Path>,
    pub sidecar: crate::sidecar_part::Options,
    /// Local storage the staged copies are made in.
    pub scratch: &'a Path,
    /// Called at named moments (`between-stores`: the Sidecar is staged
    /// and the Library not yet), for a test to hold the writer there.
    pub at: &'a dyn Fn(&str),
}

/// A backup's contents, staged: immutable copies, removed when dropped.
pub struct Staged {
    dir: tempfile::TempDir,
    location: rbl_db::LibraryLocation,
    created_at: u64,
    counts: crate::summary::Counts,
    library_files: Vec<String>,
    /// The Sidecar part: its folders and its files' names in the archive.
    sidecar: Option<(Vec<String>, Vec<String>)>,
    /// The Library's `localUpdateCount` in the snapshot: every change to
    /// the Library moves it.
    pub update_count: Option<i64>,
}

/// Where and how to publish.
pub struct Publish<'a> {
    /// The backup folder; made if missing.
    pub destination: &'a Path,
    /// Asked before every write to `destination`.
    pub check: &'a dyn Fn() -> io::Result<()>,
    /// Recorded in the summary.
    pub app_version: Option<String>,
}

/// A backup written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Made {
    pub path: PathBuf,
    /// Unix milliseconds, as in the manifest.
    pub created_at: u64,
    /// The archive's size on disk.
    pub bytes: u64,
}

const DATABASE: &str = "master.db";
const SIDECAR: &str = "sidecar-part";

/// Refuses a backup folder inside what a backup reads: the analysis or
/// artwork tree, or the Sidecar's folder, where the archive being written
/// would be backed up into itself (as the window's backup refuses).
///
/// # Errors
///
/// For such a folder.
pub fn refuse_overlap(
    location: &rbl_db::LibraryLocation,
    sidecar_dir: Option<&Path>,
    destination: &Path,
) -> Result<()> {
    let resolved = resolved_up(destination);
    let mut read = vec![crate::analysis_dir(location), crate::artwork_dir(location)];
    read.extend(sidecar_dir.map(Path::to_path_buf));
    for source in read {
        let source = resolved_up(&source);
        if resolved.starts_with(&source) {
            return Err(refused(format!(
                "The backup folder {} is inside {}, which the backup copies. Choose a backup folder outside the Library's analysis and artwork folders and the Sidecar.",
                destination.display(),
                source.display()
            )));
        }
    }
    Ok(())
}

/// `path` with its longest existing part resolved (links followed).
fn resolved_up(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut existing = path.to_path_buf();
    loop {
        if let Ok(found) = existing.canonicalize() {
            let mut full = found;
            for part in rest.iter().rev() {
                full.push(part);
            }
            return full;
        }
        let Some(name) = existing.file_name().map(std::ffi::OsStr::to_os_string) else {
            return path.to_path_buf();
        };
        rest.push(name);
        if !existing.pop() {
            return path.to_path_buf();
        }
    }
}

/// Stages one backup of `request.location`. Hold the Library's edit gate
/// and the analysis-file lock while it runs, and only then.
///
/// # Errors
///
/// When anything cannot be read, snapshotted or copied: nothing is kept.
pub fn stage(request: &Stage<'_>) -> Result<Staged> {
    let location = request.location;
    fs::create_dir_all(request.scratch)?;
    let dir = tempfile::Builder::new()
        .prefix("run-backup-")
        .tempdir_in(request.scratch)?;
    let created_at = rbl_core::time::unix_millis();
    // The Sidecar first, then the Library.
    let sidecar = match request.sidecar_dir {
        Some(sidecar_dir) => {
            let checked = crate::sidecar_part::Destination::check(location, sidecar_dir)?;
            match crate::sidecar_part::collect(&checked.path, request.sidecar, &mut || true)? {
                Some(part) => {
                    let root = dir.path().join(SIDECAR);
                    let stems = format!(
                        "{}/{}/",
                        crate::sidecar_part::ARCHIVE_FOLDER,
                        crate::sidecar_part::STEMS_FOLDER
                    );
                    let mut names = Vec::new();
                    for item in &part.files {
                        // Stem audio is large and replaced by rename: linked.
                        place(
                            &item.source,
                            &root.join(&item.name),
                            item.name.starts_with(&stems),
                        )?;
                        names.push(item.name.clone());
                    }
                    Some((part.directories.clone(), names))
                }
                None => None,
            }
        }
        None => None,
    };
    (request.at)("between-stores");
    let local_db = dir.path().join(DATABASE);
    let snapshot = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly)?;
    snapshot
        .connection()
        .execute("VACUUM main INTO ?1", [local_db.to_string_lossy().as_ref()])?;
    let counts = crate::summary::count(snapshot.connection())?;
    let update_count = snapshot
        .connection()
        .query_row(
            "SELECT int_1 FROM agentRegistry WHERE registry_id = 'localUpdateCount'",
            [],
            |r| r.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten();
    drop(snapshot);
    let root = location
        .master_db
        .parent()
        .ok_or_else(|| refused("Invalid database path"))?;
    let library_files: Vec<String> = crate::LIBRARY_FILES
        .into_iter()
        .filter(|name| root.join(name).is_file())
        .map(str::to_owned)
        .collect();
    for name in &library_files {
        place(
            &root.join(name),
            &dir.path().join("library").join(name),
            false,
        )?;
    }
    for (tree, prefix) in [
        (crate::analysis_dir(location), "analysis"),
        (crate::artwork_dir(location), "artwork"),
    ] {
        stage_tree(&tree, &dir.path().join(prefix))?;
    }
    Ok(Staged {
        dir,
        location: location.clone(),
        created_at,
        counts,
        library_files,
        sidecar,
        update_count,
    })
}

/// A staged copy of `source` at `target`: a hard link when `link` and the
/// file system allows, else a copy. Symbolic links are refused.
fn place(source: &Path, target: &Path, link: bool) -> Result<()> {
    if fs::symlink_metadata(source)?.file_type().is_symlink() {
        return Err(refused(format!(
            "{} is a symbolic link; a backup does not follow links.",
            source.display()
        )));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    if link && fs::hard_link(source, target).is_ok() {
        return Ok(());
    }
    fs::copy(source, target)?;
    Ok(())
}

/// The tree at `tree`, staged at `target` (an empty folder when missing).
fn stage_tree(tree: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    if !tree.exists() {
        return Ok(());
    }
    let mut pending = vec![tree.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(tree)
                .map_err(|_| refused("A file outside the tree was listed in it."))?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(refused(format!(
                    "{} is a symbolic link; a backup does not follow links.",
                    path.display()
                )));
            }
            if kind.is_dir() {
                fs::create_dir_all(target.join(relative))?;
                pending.push(path);
            } else {
                place(&path, &target.join(relative), true)?;
            }
        }
    }
    Ok(())
}

/// Writes the archive of `staged` into the backup folder.
///
/// # Errors
///
/// When anything cannot be checked or written, or `check` refuses: nothing
/// is left under the backup's own name.
pub fn publish(staged: &Staged, request: &Publish<'_>) -> Result<Made> {
    validate(&staged.dir.path().join(DATABASE), &staged.location)?;
    (request.check)()?;
    rbl_core::durable::create_dir_all(request.destination)?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let short = id.get(..8).unwrap_or(&id);
    let target = request.destination.join(format!(
        "rbexport-{}-run-{short}.zip",
        rbl_core::time::local_backup_stamp()
    ));
    let partial = request.destination.join(format!(".partial-{id}.zip"));
    if let Err(error) = written(staged, request, &partial) {
        return Err(left(request, &partial, error));
    }
    (request.check)()?;
    if let Err(error) =
        rbl_core::durable::persist_new(tempfile::TempPath::try_from_path(&partial)?, &target)
    {
        return Err(left(request, &partial, error.into()));
    }
    (request.check)()?;
    rbl_core::durable::sync_dir(request.destination)?;
    let bytes = fs::metadata(&target)?.len();
    Ok(Made {
        path: target,
        created_at: staged.created_at,
        bytes,
    })
}

/// Removes a backup this module published, when a write is still allowed:
/// one taken for a Run that then had to be taken again.
///
/// # Errors
///
/// When `check` refuses or the file cannot be removed: it is left.
pub fn discard(made: &Path, check: &dyn Fn() -> io::Result<()>) -> Result<()> {
    check()?;
    fs::remove_file(made)?;
    if let Some(parent) = made.parent() {
        rbl_core::durable::sync_dir(parent)?;
    }
    Ok(())
}

/// Removes what a failed backup left, when a write is still allowed.
fn left(request: &Publish<'_>, partial: &Path, error: Error) -> Error {
    if !partial.exists() {
        return error;
    }
    if (request.check)().is_ok() && fs::remove_file(partial).is_ok() {
        return error;
    }
    refused(format!(
        "{error} The unfinished backup {} was left; delete it.",
        partial.display()
    ))
}

fn written(staged: &Staged, request: &Publish<'_>, partial: &Path) -> Result<()> {
    let root = staged.dir.path();
    (request.check)()?;
    let file = File::options().write(true).create_new(true).open(partial)?;
    let mut zip = ZipWriter::new(BufWriter::with_capacity(
        1 << 20,
        Checked {
            file,
            check: request.check,
        },
    ));
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    let database_bytes = fs::metadata(root.join(DATABASE))?.len();
    let mut bytes = add_file(&mut zip, DATABASE, &root.join(DATABASE), options)?;
    let mut library_bytes = 0;
    for name in &staged.library_files {
        library_bytes += add_file(&mut zip, name, &root.join("library").join(name), options)?;
    }
    bytes += library_bytes;
    for prefix in ["analysis", "artwork"] {
        bytes += add_tree(&mut zip, &root.join(prefix), prefix, options)?;
    }
    if let Some((directories, names)) = &staged.sidecar {
        for directory in directories {
            zip.add_directory(format!("{directory}/"), SimpleFileOptions::default())?;
        }
        // `sidecar.json` is among them.
        for name in names {
            bytes += add_file(&mut zip, name, &root.join(SIDECAR).join(name), options)?;
        }
    }
    let mut measured = crate::sizes::Measurement::default();
    for (prefix, is_artwork) in [("analysis", false), ("artwork", true)] {
        crate::sizes::measure_tree(
            &root.join(prefix),
            is_artwork,
            &mut measured,
            &mut || Ok(()),
        )?;
    }
    measured.sizes.database = database_bytes + library_bytes;
    let summary = crate::summary::Summary::new(
        staged.created_at,
        request.app_version.clone(),
        measured,
        staged.counts,
    );
    add_bytes(
        &mut zip,
        crate::summary::NAME,
        &serde_json::to_vec(&summary)?,
        options,
    )?;
    let manifest = crate::manifest::Manifest {
        version: crate::manifest::VERSION,
        library: staged.location.master_db.clone(),
        created_at: staged.created_at,
        bytes,
        includes_artwork: true,
        library_files: staged.library_files.clone(),
    };
    add_bytes(
        &mut zip,
        crate::manifest::NAME,
        &serde_json::to_vec(&manifest)?,
        options,
    )?;
    let mut out = zip.finish()?;
    out.flush()?;
    let checked = out.into_inner().map_err(io::IntoInnerError::into_error)?;
    (request.check)()?;
    rbl_core::durable::flush(&checked.file)?;
    Ok(())
}

type Zip<'a> = ZipWriter<BufWriter<Checked<'a>>>;

fn add_bytes(
    zip: &mut Zip<'_>,
    name: &str,
    bytes: &[u8],
    options: SimpleFileOptions,
) -> Result<()> {
    zip.start_file(name, options)?;
    zip.write_all(bytes)?;
    Ok(())
}

fn add_file(
    zip: &mut Zip<'_>,
    name: &str,
    source: &Path,
    options: SimpleFileOptions,
) -> Result<u64> {
    zip.start_file(name, options)?;
    let mut file = File::open(source)?;
    Ok(io::copy(&mut file, zip)?)
}

/// A staged tree under `prefix/`, folders included, in name order.
fn add_tree(
    zip: &mut Zip<'_>,
    tree: &Path,
    prefix: &str,
    options: SimpleFileOptions,
) -> Result<u64> {
    zip.add_directory(format!("{prefix}/"), SimpleFileOptions::default())?;
    let mut bytes = 0;
    let mut pending = vec![tree.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut entries: Vec<_> = fs::read_dir(&directory)?.collect::<io::Result<_>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(tree)
                .map_err(|_| refused("A file outside the tree was listed in it."))?
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type()?.is_dir() {
                zip.add_directory(
                    format!("{prefix}/{relative}/"),
                    SimpleFileOptions::default(),
                )?;
                pending.push(path);
            } else {
                bytes += add_file(zip, &format!("{prefix}/{relative}"), &path, options)?;
            }
        }
    }
    Ok(bytes)
}

/// The snapshot opens with the Library's key and is not worse than the
/// Library it was copied from (the window's rule, #805).
fn validate(path: &Path, location: &rbl_db::LibraryLocation) -> Result<()> {
    let mut copy = location.clone();
    copy.master_db = path.to_path_buf();
    copy.is_real_install = false;
    let staged = problems(&rbl_db::Library::open(copy, rbl_db::OpenMode::ReadOnly)?)?;
    if staged.is_empty() {
        return Ok(());
    }
    let original = problems(&rbl_db::Library::open(
        location.clone(),
        rbl_db::OpenMode::ReadOnly,
    )?)?;
    match staged.iter().find(|problem| !original.contains(problem)) {
        None => Ok(()),
        Some(problem) => Err(refused(format!(
            "The backup copy of the library database (master.db) failed its integrity check: {problem}"
        ))),
    }
}

fn problems(db: &rbl_db::Library) -> Result<Vec<String>> {
    let mut statement = db.connection().prepare("PRAGMA quick_check")?;
    let rows = statement.query_map([], |r| r.get::<_, String>(0))?;
    let mut found = Vec::new();
    for row in rows {
        let row = row?;
        if row != "ok" {
            found.push(row);
        }
    }
    Ok(found)
}

/// The archive file, asking before every write.
struct Checked<'a> {
    file: File,
    check: &'a dyn Fn() -> io::Result<()>,
}

impl Write for Checked<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (self.check)()?;
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Checked<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.file.seek(to)
    }
}

impl Read for Checked<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}
