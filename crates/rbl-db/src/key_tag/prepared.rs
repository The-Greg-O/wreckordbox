//! Prepare audio outside the Library gate; publish and roll back using only
//! metadata and atomic renames. The original bytes remain available until Drop.
//! Unsupported exchange falls back to rename-over-original. A separately synced
//! rollback copy is prepared outside the Library gate for that case.
use super::{
    can_write, format_of, keep_created, keep_owner, links, read_key, sync_parent, temp_beside,
    verify, write_flac, write_id3, Format, KeySwap, KeyTagError, Result,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

mod cleanup;
mod staging;
mod verification;
/// Explicit cleanup entry point. The caller must authorize this location;
/// preparation also sweeps stale copies under its authorization callback.
pub use cleanup::cleanup_stale;
pub use verification::Verified;

pub enum Outcome {
    Ready(Box<Prepared>),
    Skipped(KeySwap),
}

#[derive(Clone, Copy)]
enum Expected<'a> {
    Any,
    Value(Option<&'a str>),
}

pub struct Prepared {
    requested: PathBuf,
    path: PathBuf,
    temp: PathBuf,
    backup: PathBuf,
    backup_stamp: Option<fs::Metadata>,
    exchanged: bool,
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
    prepare_checked(path, expected, key, || Ok(()))
}

/// As prepare, with a per-write authorization check for stale cleanup and staging.
/// The caller holds its location authorization while this runs.
pub fn prepare_checked(
    path: &Path,
    expected: Option<Option<&str>>,
    key: Option<&str>,
    authorize: impl Fn() -> io::Result<()>,
) -> Result<Outcome> {
    prepare_inner(
        path,
        expected.map_or(Expected::Any, Expected::Value),
        key,
        authorize,
        None,
    )
}

/// Simulates a filesystem without exchange during off-gate preparation.
#[doc(hidden)]
pub fn prepare_with_exchange_error_for_test(
    path: &Path,
    expected: Option<Option<&str>>,
    key: Option<&str>,
    kind: io::ErrorKind,
) -> Result<Outcome> {
    assert!(crate::test_mode());
    prepare_inner(
        path,
        expected.map_or(Expected::Any, Expected::Value),
        key,
        || Ok(()),
        Some(kind),
    )
}

fn prepare_inner(
    path: &Path,
    expected: Expected<'_>,
    key: Option<&str>,
    authorize: impl Fn() -> io::Result<()>,
    exchange_error: Option<io::ErrorKind>,
) -> Result<Outcome> {
    authorize()?;
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
    if matches!(expected, Expected::Value(value) if value != before.as_deref()) {
        return Ok(Outcome::Skipped(KeySwap::Changed { found: before }));
    }
    cleanup::cleanup_stale_checked(&path, &authorize)?;
    let temp = temp_beside(&path);
    let backup = temp_beside(&path);
    let mut prepared = Prepared {
        requested,
        path,
        temp,
        backup,
        backup_stamp: None,
        exchanged: false,
        before,
        after: key.map(str::to_owned),
        original,
        published: None,
        staged: None,
        cleanup: true,
    };
    let staged = (|| -> Result<()> {
        // Same rewrite/verification as swap_key, without any Library lock.
        authorize()?;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&prepared.temp)?;
        fs::copy(&prepared.path, &prepared.temp)?;
        prepared.stage_backup(&authorize, exchange_error)?;
        authorize()?;
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
        Ok(())
    })();
    if let Err(why) = staged {
        if authorize().is_err() {
            prepared.retain_temporary();
        }
        return Err(why);
    }
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

    /// Publish while the caller owns its edit/authorization guards. Only the
    /// same canonical file waits: validation and replacement must be indivisible
    /// to another in-process publisher, including the legacy writer.
    pub fn publish(&mut self) -> Result<()> {
        self.publish_checked(|| {})
    }

    /// Controlled fixture interleaving after validation, before publication.
    #[doc(hidden)]
    pub fn publish_held_for_test(&mut self, checked: impl FnOnce()) -> Result<()> {
        assert!(crate::test_mode());
        self.publish_checked(checked)
    }

    fn publish_checked(&mut self, checked: impl FnOnce()) -> Result<()> {
        let writing = super::file_lock::for_path(&self.path);
        let _writer = writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.check()?;
        let next = fs::metadata(&self.temp)?;
        if !self
            .staged
            .as_ref()
            .is_some_and(|staged| same(staged, &next))
        {
            return Err(changed());
        }
        checked();
        if self.backup_stamp.is_some() {
            // Exchange was unsupported in the off-gate probe; the synced backup
            // was staged there. Publication never copies audio under the gate.
            self.check()?;
            fs::rename(&self.temp, &self.path)?;
        } else {
            // If filesystem capability changed after preparation, fail closed:
            // no unprepared fallback or audio copy inside the Library gate.
            exchange(&self.temp, &self.path)?;
            self.exchanged = true;
        }
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
        let writing = super::file_lock::for_path(&self.path);
        let _writer = writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if fs::canonicalize(&self.requested)? != self.path
            || !same(published, &fs::metadata(&self.path)?)
        {
            return Err(changed());
        }
        if self.exchanged {
            exchange(&self.temp, &self.path)?;
        } else {
            if !self
                .backup_stamp
                .as_ref()
                .is_some_and(|stamp| fs::metadata(&self.backup).is_ok_and(|now| same(stamp, &now)))
            {
                return Err(changed());
            }
            fs::rename(&self.backup, &self.path)?;
        }
        self.published = None;
        sync_parent(&self.path);
        Ok(())
    }

    pub fn is_published(&self) -> bool {
        self.published.is_some()
    }

    /// Authorization was lost: retain the copies. Explicit `cleanup_stale` or
    /// a later authorized prepare can sweep them once old and this process has
    /// exited. Live processes' plans are never swept.
    pub fn retain_temporary(&mut self) {
        self.cleanup = false;
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = fs::remove_file(&self.temp);
            let _ = fs::remove_file(&self.backup);
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
