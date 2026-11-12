//! Putting the finished file at the destination, and nothing else anywhere.
//!
//! Two promises are kept here. **A render removes only what it made:** the
//! temporary file is created exclusively, under a fresh name if the first is
//! taken, and only a name this render created is ever cleaned up. **A render
//! never replaces a file:** the finished file is moved into place by an
//! operation that fails, in one step, if the destination's name is taken:
//! a rename that refuses to replace where the filesystem has one, a hard
//! link where it does not. Where it has neither (FAT and exFAT on some
//! systems) the render fails with [`RenderError::NoReplaceUnsupported`]. A
//! check followed by an ordinary rename is not a substitute and is never
//! made: two renders can both pass the check.
//!
//! [`Filesystem`] is the seam: [`Real`] in use, and in tests a stand-in for
//! a filesystem that lacks one operation or the other, or one that lets a
//! competitor in at the moment of publishing.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use crate::RenderError;

/// The few things a render asks of a filesystem beyond writing to a file it
/// has open.
pub trait Filesystem {
    /// Creates `path` for writing, failing with `AlreadyExists` if anything
    /// is there.
    fn create_new(&self, path: &Path) -> io::Result<File>;

    /// Moves `from` to `to` in one step, failing with `AlreadyExists` if
    /// anything is at `to`.
    fn rename_noreplace(&self, from: &Path, to: &Path) -> io::Result<()>;

    /// Gives `from` the second name `to`, failing with `AlreadyExists` if
    /// anything is at `to`.
    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()>;
}

/// The real one.
pub struct Real;

impl Filesystem for Real {
    fn create_new(&self, path: &Path) -> io::Result<File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
    fn rename_noreplace(&self, from: &Path, to: &Path) -> io::Result<()> {
        use rustix::fs::{renameat_with, RenameFlags, CWD};
        renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(io::Error::from)
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    fn rename_noreplace(&self, _from: &Path, _to: &Path) -> io::Result<()> {
        // Nothing portable; the hard link does the same job.
        Err(io::ErrorKind::Unsupported.into())
    }

    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::hard_link(from, to)
    }
}

/// How many fresh names are tried for the temporary file before giving up.
const NAMES: u32 = 16;

/// The file being written, under a name that is not the destination's and
/// that this render created. Dropped without [`Part::commit`], it is
/// removed.
pub(crate) struct Part<'a> {
    files: &'a dyn Filesystem,
    /// A name this render created, exclusively: the only thing it may remove.
    path: PathBuf,
    /// Whether the file has left this name for the destination's.
    moved: bool,
}

impl<'a> Part<'a> {
    /// Creates the temporary file in the destination's own folder, so that
    /// putting it in place never crosses a filesystem.
    ///
    /// A name that is already taken is left exactly as it is and another is
    /// tried: a `Part` exists only once a name has been created, so there is
    /// never a name to clean up that was someone else's.
    pub(crate) fn create(
        files: &'a dyn Filesystem,
        destination: &Path,
    ) -> Result<(Self, File), RenderError> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = destination.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the destination has no file name",
            )
        })?;
        for _ in 0..NAMES {
            let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // The clock as well as the count, so a name left by an earlier
            // process with the same id is not walked into again and again.
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.subsec_nanos());
            let mut part = std::ffi::OsString::from(".");
            part.push(name);
            part.push(format!(".{}-{unique}-{nanos:08x}.part", std::process::id()));
            let path = destination.with_file_name(part);
            match files.create_new(&path) {
                Ok(file) => {
                    return Ok((
                        Self {
                            files,
                            path,
                            moved: false,
                        },
                        file,
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "every temporary name tried beside the destination was taken",
        )
        .into())
    }

    /// Puts the finished file at `destination`, or fails leaving whatever is
    /// there untouched.
    pub(crate) fn commit(mut self, destination: &Path) -> Result<(), RenderError> {
        let taken = || RenderError::DestinationExists(destination.to_path_buf());
        match self.files.rename_noreplace(&self.path, destination) {
            Ok(()) => {
                self.moved = true;
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(taken()),
            Err(error) if not_offered(&error) => {}
            Err(error) => return Err(error.into()),
        }
        // A second name for the same file, which also fails if the name is
        // taken; the temporary name is then removed as after any render.
        match self.files.hard_link(&self.path, destination) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(taken()),
            Err(error) if not_offered(&error) => {
                Err(RenderError::NoReplaceUnsupported(destination.to_path_buf()))
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for Part<'_> {
    fn drop(&mut self) {
        if !self.moved {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Whether the filesystem said it does not have the operation, as opposed
/// to failing it.
///
/// `EINVAL` is what Linux answers for a rename flag a filesystem does not
/// know, `ENOSYS` and `ENOTSUP` what older kernels and macOS answer, and
/// `EPERM` (1) what a hard link gets on FAT and exFAT, and the code Windows
/// uses for a function a volume lacks.
fn not_offered(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
    ) || error.raw_os_error() == Some(1)
}
