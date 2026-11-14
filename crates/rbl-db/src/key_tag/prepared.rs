//! Prepare audio outside the Library gate; publish and roll back using only
//! metadata and atomic renames. The original bytes remain available until Drop.
//! Publication currently needs Linux/macOS atomic exchange; other platforms
//! return Unsupported before replacing the original.
use super::{
    can_write, format_of, keep_created, keep_owner, links, read_key, sync_parent, temp_beside,
    verify, write_flac, write_id3, Format, KeySwap, KeyTagError, Result, WRITING,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub enum Outcome {
    Ready(Box<Prepared>),
    Skipped(KeySwap),
}

pub struct Prepared {
    requested: PathBuf,
    path: PathBuf,
    temp: PathBuf,
    before: Option<String>,
    after: Option<String>,
    original: fs::Metadata,
    published: Option<fs::Metadata>,
    staged: Option<fs::Metadata>,
    cleanup: bool,
}

fn changed() -> KeyTagError {
    KeyTagError::Io(io::Error::other(
        "The audio file changed while its key tag was prepared",
    ))
}
fn same(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    let basic = a.len() == b.len()
        && a.modified().ok() == b.modified().ok()
        && a.permissions() == b.permissions()
        && links(a) == links(b);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        basic
            && a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        basic && a.created().ok() == b.created().ok()
    }
}

/// No original file is changed here. Call while authorized to create the
/// temporary beside it; the caller must authorize publication separately.
pub fn prepare(path: &Path, expected: Option<Option<&str>>, key: Option<&str>) -> Result<Outcome> {
    let requested = path.to_path_buf();
    let path = fs::canonicalize(path)?;
    let original = fs::metadata(&path)?;
    if !original.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file").into());
    }
    if !can_write(&path, &original)? {
        return Ok(Outcome::Skipped(KeySwap::ReadOnly));
    }
    let Some(format) = format_of(&path)? else {
        return Ok(Outcome::Skipped(KeySwap::Unsupported));
    };
    if links(&original) > 1 {
        return Err(KeyTagError::Tag(
            "the file has other hard links, which a replacement would split off".into(),
        ));
    }
    let before = read_key(&path)?;
    if expected.is_some_and(|expected| expected != before.as_deref()) {
        return Ok(Outcome::Skipped(KeySwap::Changed { found: before }));
    }
    let temp = temp_beside(&path);
    let mut prepared = Prepared {
        requested,
        path,
        temp,
        before,
        after: key.map(str::to_owned),
        original,
        published: None,
        staged: None,
        cleanup: true,
    };
    // Same rewrite/verification as swap_key, without any Library lock.
    fs::copy(&prepared.path, &prepared.temp)?;
    match format {
        Format::Mp3 | Format::Aiff => write_id3(&prepared.path, &prepared.temp, format, key)?,
        Format::Flac => write_flac(&prepared.path, &prepared.temp, key)?,
    }
    verify(&prepared.path, &prepared.temp, format, key)?;
    let file = fs::OpenOptions::new().write(true).open(&prepared.temp)?;
    keep_created(&file, &prepared.original);
    keep_owner(&file, &prepared.original)?;
    file.sync_all()?;
    prepared.staged = Some(file.metadata()?);
    prepared.check()?;
    Ok(Outcome::Ready(Box::new(prepared)))
}

impl Prepared {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn before(&self) -> Option<&str> {
        self.before.as_deref()
    }
    pub fn after(&self) -> Option<&str> {
        self.after.as_deref()
    }

    /// Metadata only. A symlink retarget, replacement, chmod or in-place edit
    /// invalidates the plan, even when it leaves the same key text behind.
    pub fn check(&self) -> Result<()> {
        if fs::canonicalize(&self.requested)? != self.path
            || !same(&self.original, &fs::metadata(&self.path)?)
        {
            return Err(changed());
        }
        Ok(())
    }

    /// Publish while the caller owns its edit/authorization guards. Never wait
    /// under the Library gate for a legacy writer doing expensive audio I/O.
    pub fn publish(&mut self) -> Result<()> {
        let _writer = WRITING.try_lock().map_err(|_| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "Another audio key writer is publishing",
            )
        })?;
        self.check()?;
        let next = fs::metadata(&self.temp)?;
        if !self
            .staged
            .as_ref()
            .is_some_and(|staged| same(staged, &next))
        {
            return Err(changed());
        }
        exchange(&self.temp, &self.path)?;
        // Publication succeeded even if a later metadata read fails. Keep the
        // fallback identity for rollback and never report an untouched file.
        self.published = Some(fs::metadata(&self.path).unwrap_or(next));
        sync_parent(&self.path);
        Ok(())
    }

    /// Restore the original bytes without rereading/copying audio. Refuses to
    /// replace a subsequent writer's file. The caller retains its write guards.
    pub fn rollback(&mut self) -> Result<()> {
        let Some(published) = &self.published else {
            return Ok(());
        };
        let _writer = WRITING.try_lock().map_err(|_| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "Another audio key writer is publishing",
            )
        })?;
        if fs::canonicalize(&self.requested)? != self.path
            || !same(published, &fs::metadata(&self.path)?)
        {
            return Err(changed());
        }
        exchange(&self.temp, &self.path)?;
        self.published = None;
        sync_parent(&self.path);
        Ok(())
    }

    /// An authorization was lost: leave the temporary for an explicit later
    /// cleanup instead of writing to a protected or replaced Stick from Drop.
    pub fn retain_temporary(&mut self) {
        self.cleanup = false;
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn exchange(a: &Path, b: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        a,
        rustix::fs::CWD,
        b,
        rustix::fs::RenameFlags::EXCHANGE,
    )
    .map_err(Into::into)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn exchange(_a: &Path, _b: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Atomic audio tag exchange is not available on this platform",
    ))
}
