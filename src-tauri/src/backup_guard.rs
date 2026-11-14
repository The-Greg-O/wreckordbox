//! Stick Protection at the backup's own write boundaries (wbx-45.14,
//! round 1).
//!
//! A backup to a Stick writes from several threads (the compression
//! workers) and through `ZipWriter`, which writes again when it is dropped
//! unfinished (it finalizes the archive). Checking in the progress
//! consumer is not enough: a worker can write ahead of it. So every write to
//! a file on the backup's folder goes through [`Checked`]: the check (the
//! Stick is the same mount and still unlocked) runs before each write the
//! file receives, behind a buffer of [`CHUNK`] bytes, so at most one chunk
//! already checked is written after a refusal and nothing after it (a
//! dropped `ZipWriter`'s finalization included: its bytes are refused).
//! Creating, removing or renaming anything there checks first too.

use std::fs;
use std::io::{self, Seek, SeekFrom, Write};

/// What a write to the backup's folder checks first: `Err` stops it.
pub type WriteCheck<'a> = &'a (dyn Fn() -> io::Result<()> + Sync);

/// For a backup's local files (and tests): never refuses.
#[cfg_attr(not(test), allow(dead_code))]
pub const UNCHECKED: WriteCheck<'static> = &|| Ok(());

/// The most written to a file between two checks.
pub const CHUNK: usize = 1024 * 1024;

/// A file whose every write is checked first.
pub struct Checked<'a, W> {
    inner: W,
    check: WriteCheck<'a>,
}

impl<W: Write> Write for Checked<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (self.check)()?;
        self.inner.write(&buf[..buf.len().min(CHUNK)])
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl<W: Seek> Seek for Checked<'_, W> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.inner.seek(to)
    }
}

/// A checked, buffered writer for a new file at `path`, created only after
/// the check.
pub fn create_new<'a>(
    path: &std::path::Path,
    check: WriteCheck<'a>,
) -> io::Result<io::BufWriter<Checked<'a, fs::File>>> {
    check()?;
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    Ok(io::BufWriter::with_capacity(
        CHUNK,
        Checked { inner: file, check },
    ))
}

/// The file behind a writer from [`create_new`], every buffered byte
/// written (and checked) first, synced to disk.
pub fn finish(writer: io::BufWriter<Checked<'_, fs::File>>) -> io::Result<()> {
    let checked = writer
        .into_inner()
        .map_err(io::IntoInnerError::into_error)?;
    (checked.check)()?;
    checked.inner.sync_all()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn nothing_buffered_reaches_the_file_once_the_check_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.zip");
        let refused = AtomicBool::new(false);
        let check = || {
            if refused.load(Ordering::Acquire) {
                Err(io::Error::new(io::ErrorKind::PermissionDenied, "locked"))
            } else {
                Ok(())
            }
        };
        let mut zip = zip::ZipWriter::new(create_new(&path, &check).unwrap());
        zip.start_file("data", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&[7; 4096]).unwrap();
        let at_refusal = fs::metadata(&path).unwrap().len();
        refused.store(true, Ordering::Release);
        // Dropped unfinished: its finalization is refused, and so is the
        // buffered entry.
        drop(zip);
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            at_refusal,
            "not a byte written after the refusal"
        );
    }
}
