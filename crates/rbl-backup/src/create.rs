//! Writing a backup archive from outside the window (wbx-25: the backup a
//! large Run takes first).
//!
//! The same archive the window's Preferences › Backups writes, read by the
//! same restore (`wreckordbox backup restore`) and listed by the same lists:
//! a vacuumed snapshot of `master.db`, the analysis and artwork trees, the
//! library files beside `master.db`, the Sidecar by [`crate::sidecar_part`]'s
//! rule, [`crate::summary`] and [`crate::manifest`], under the window's
//! naming scheme (`rbexport-…zip`). What it does not carry are the restore
//! scripts the window adds (they are the app shell's); such a backup is
//! restored with `wreckordbox backup restore`.
//!
//! Order, as the window's: the Sidecar is read before the Library, so every
//! Ledger entry in the backup is of a change its Library has.
//!
//! Every write to the backup folder asks `check` first (the folder may be on
//! a Stick, whose authorization must still hold), through [`Checked`]. The
//! archive is written under a `.partial-…zip` name and renamed into place
//! complete, never replacing a file. On a failure the partial file is
//! removed when `check` still allows a write; otherwise it is left and
//! named in the error (removing it would be a write).
//!
//! The database snapshot is made in `scratch` (local storage), checked
//! there, then copied into the archive.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::{refused, Error, Result};

/// What to back up, and where.
pub struct Request<'a> {
    pub location: &'a rbl_db::LibraryLocation,
    /// The Sidecar's folder (checked against the Library), or `None` for a
    /// Library with no Sidecar part.
    pub sidecar_dir: Option<&'a Path>,
    pub sidecar: crate::sidecar_part::Options,
    /// The backup folder; made if missing.
    pub destination: &'a Path,
    /// Local storage for the database snapshot.
    pub scratch: &'a Path,
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

/// Writes one backup of `request.location`.
///
/// # Errors
///
/// When anything cannot be read, snapshotted, checked or written, or
/// `check` refuses: nothing is left under the backup's own name.
pub fn write(request: &Request<'_>) -> Result<Made> {
    (request.check)()?;
    rbl_core::durable::create_dir_all(request.destination)?;
    let created_at = rbl_core::time::unix_millis();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let short = id.get(..8).unwrap_or(&id);
    let target = request.destination.join(format!(
        "rbexport-{}-run-{short}.zip",
        rbl_core::time::local_backup_stamp()
    ));
    let partial = request.destination.join(format!(".partial-{id}.zip"));
    match written(request, &partial, created_at) {
        Ok(()) => {}
        Err(error) => return Err(left(request, &partial, error)),
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
        created_at,
        bytes,
    })
}

/// Removes what a failed backup left, when a write is still allowed.
fn left(request: &Request<'_>, partial: &Path, error: Error) -> Error {
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

fn written(request: &Request<'_>, partial: &Path, created_at: u64) -> Result<()> {
    let location = request.location;
    // The Sidecar first, then the Library.
    let sidecar = match request.sidecar_dir {
        Some(dir) => {
            let checked = crate::sidecar_part::Destination::check(location, dir)?;
            crate::sidecar_part::collect(&checked.path, request.sidecar, &mut || true)?
        }
        None => None,
    };
    fs::create_dir_all(request.scratch)?;
    let scratch = tempfile::Builder::new()
        .prefix("run-backup-")
        .tempdir_in(request.scratch)?;
    let local_db = scratch.path().join("master.db");
    let snapshot = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly)?;
    snapshot
        .connection()
        .execute("VACUUM main INTO ?1", [local_db.to_string_lossy().as_ref()])?;
    let counts = crate::summary::count(snapshot.connection())?;
    drop(snapshot);
    validate(&local_db, location)?;
    let database_bytes = fs::metadata(&local_db)?.len();

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
    let mut bytes = add_file(&mut zip, "master.db", &local_db, options)?;
    drop(scratch);
    let root = location
        .master_db
        .parent()
        .ok_or_else(|| refused("Invalid database path"))?;
    let library_files: Vec<String> = crate::LIBRARY_FILES
        .into_iter()
        .filter(|name| root.join(name).is_file())
        .map(str::to_owned)
        .collect();
    let mut library_bytes = 0;
    for name in &library_files {
        library_bytes += add_file(&mut zip, name, &root.join(name), options)?;
    }
    bytes += library_bytes;
    let analysis = crate::analysis_dir(location);
    let artwork = crate::artwork_dir(location);
    for (tree, prefix) in [(&analysis, "analysis"), (&artwork, "artwork")] {
        bytes += add_tree(&mut zip, tree, prefix, options)?;
    }
    if let Some(part) = &sidecar {
        for directory in &part.directories {
            zip.add_directory(format!("{directory}/"), SimpleFileOptions::default())?;
        }
        for item in &part.files {
            // `sidecar.json` is among them.
            bytes += add_file(&mut zip, &item.name, &item.source, options)?;
        }
    }
    let mut measured = crate::sizes::Measurement::default();
    for (tree, is_artwork) in [(&analysis, false), (&artwork, true)] {
        crate::sizes::measure_tree(tree, is_artwork, &mut measured, &mut || Ok(()))?;
    }
    measured.sizes.database = database_bytes + library_bytes;
    let summary =
        crate::summary::Summary::new(created_at, request.app_version.clone(), measured, counts);
    add_bytes(
        &mut zip,
        crate::summary::NAME,
        &serde_json::to_vec(&summary)?,
        options,
    )?;
    let manifest = crate::manifest::Manifest {
        version: crate::manifest::VERSION,
        library: location.master_db.clone(),
        created_at,
        bytes,
        includes_artwork: true,
        library_files,
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
    let meta = fs::symlink_metadata(source)?;
    if meta.file_type().is_symlink() {
        return Err(refused(format!(
            "{} is a symbolic link; a backup does not follow links.",
            source.display()
        )));
    }
    zip.start_file(name, options)?;
    let mut file = File::open(source)?;
    Ok(io::copy(&mut file, zip)?)
}

/// A tree under `prefix/`, folders included; a missing tree is an empty
/// folder, as the window writes it.
fn add_tree(
    zip: &mut Zip<'_>,
    tree: &Path,
    prefix: &str,
    options: SimpleFileOptions,
) -> Result<u64> {
    zip.add_directory(format!("{prefix}/"), SimpleFileOptions::default())?;
    if !tree.exists() {
        return Ok(0);
    }
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
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(refused(format!(
                    "{} is a symbolic link; a backup does not follow links.",
                    path.display()
                )));
            }
            if kind.is_dir() {
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

/// The archive file, asking before every write and seek that may write.
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
