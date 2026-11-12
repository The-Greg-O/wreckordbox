//! Stick Protection (wbx-45.11): nothing writes to a Stick, or settles an
//! unfinished write on it, unless the DJ has unlocked that Stick.
//!
//! The backstop under every Stick write, whoever asks (the window in either
//! layout, the Library service, the command line, a script, background
//! work). The window's staged plan and hold are the per-operation
//! confirmation on top of it; this is the guarantee.
//!
//! - **Protected by default.** A Stick is unlocked only while an unlock
//!   recorded for it matches the mount it is on now: its path, the mounted
//!   folder's device and inode, and (on Linux) the kernel's mount id. So an
//!   unlock ends when the Stick is ejected, pulled out or put back, or when
//!   the DJ locks it again; nothing is remembered across a re-insert.
//! - **Unlocking names the Stick.** A Spare stick (it carries
//!   [`SPARE_MARKER`]) unlocks with a plain confirmation. Any other Stick is
//!   treated as a Gig stick: its name must be typed, and is checked here.
//! - **Kept like Library Protection** (`rbl_library::protection`): one
//!   file, [`FILE`], in the state folder the window and the service share;
//!   every change under an exclusive lock on [`LOCK_FILE`], written to a
//!   temporary file and renamed over the old, with a revision higher than
//!   any before. A missing, unreadable or corrupt file unlocks nothing.
//!
//! Which destinations are on a Stick is [`stick_root_of`]: a mounted
//! device's folder, or (for tests) a folder carrying [`STAND_IN_MARKER`].
//! An ordinary folder is not a Stick and is never refused here.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The record's name inside the shared state folder.
pub const FILE: &str = "stick-protection.json";
/// The file every writer of [`FILE`] holds an exclusive lock on.
pub const LOCK_FILE: &str = "stick-protection.lock";
/// The marker at a Stick's root that declares it a Spare stick.
pub const SPARE_MARKER: &str = ".rbxport-disposable";
/// A folder carrying this file is treated as a mounted Stick: what tests
/// use to stand in for one. A real Stick never needs it.
pub const STAND_IN_MARKER: &str = ".rbxport-stands-for-stick";
/// The stable error code of a write refused on a protected Stick.
pub const CODE: &str = "stick_protected";

/// One mount of a Stick: an unlock holds for exactly this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub path: PathBuf,
    pub dev: u64,
    pub ino: u64,
    /// The kernel's id of this mount (Linux), new for every mount.
    pub mount_id: Option<u64>,
}

/// Whether a Stick is protected, as the DJ is shown it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub protected: bool,
    /// It carries the Spare stick marker: it unlocks with a confirmation.
    pub spare: bool,
    /// The name the DJ types to unlock a Gig stick: the mounted folder's.
    pub name: String,
    pub revision: u64,
}

/// Why an unlock was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockError {
    /// A Gig stick: its name must be typed.
    NameRequired { name: String },
    /// The typed name is not the Stick's.
    NameMismatch { name: String },
    /// There is no Stick there.
    NotFound,
    /// The record could not be written; the Stick stays protected.
    Io(String),
}

impl std::fmt::Display for UnlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameRequired { name } => write!(
                f,
                "{name} is a Gig stick: type its name, {name}, to unlock it."
            ),
            Self::NameMismatch { name } => write!(
                f,
                "That is not this Stick's name. Type {name} exactly to unlock it."
            ),
            Self::NotFound => f.write_str("There is no Stick there."),
            Self::Io(why) => write!(
                f,
                "The Stick could not be unlocked, so it stays protected: {why}"
            ),
        }
    }
}

impl std::error::Error for UnlockError {}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    unlocked: Vec<Mount>,
}

/// The Stick's name: its mounted folder's.
#[must_use]
pub fn name_of(root: &Path) -> String {
    root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Whether the Stick carries the Spare stick marker.
#[must_use]
pub fn is_spare(root: &Path) -> bool {
    root.join(SPARE_MARKER).is_file()
}

/// The mount the Stick at `root` is on now.
///
/// # Errors
///
/// When nothing is mounted there.
pub fn mount_of(root: &Path) -> std::io::Result<Mount> {
    let path = root.canonicalize()?;
    let meta = std::fs::metadata(&path)?;
    let (dev, ino) = identity(&meta);
    Ok(Mount {
        mount_id: mount_id(&path),
        path,
        dev,
        ino,
    })
}

#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn identity(meta: &std::fs::Metadata) -> (u64, u64) {
    let created = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    (0, created)
}

/// The id of the mount whose mount point is the longest prefix of `path`.
fn mount_id(path: &Path) -> Option<u64> {
    let table = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    let mut best: Option<(usize, u64)> = None;
    for line in table.lines() {
        let mut fields = line.split(' ');
        let id = fields.next()?.parse::<u64>().ok()?;
        let point = fields.nth(3)?.replace("\\040", " ");
        if path.starts_with(&point) && best.is_none_or(|(len, _)| point.len() > len) {
            best = Some((point.len(), id));
        }
    }
    best.map(|(_, id)| id)
}

fn read_stored(state_dir: &Path) -> Stored {
    std::fs::read(state_dir.join(FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Whether the Stick at `root` is protected now, read from `state_dir`.
#[must_use]
pub fn status(state_dir: &Path, root: &Path) -> Status {
    let stored = read_stored(state_dir);
    let unlocked = mount_of(root).is_ok_and(|now| stored.unlocked.contains(&now));
    Status {
        protected: !unlocked,
        spare: is_spare(root),
        name: name_of(root),
        revision: stored.revision,
    }
}

/// Whether a write to the Stick at `root` may go ahead.
#[must_use]
pub fn is_unlocked(state_dir: &Path, root: &Path) -> bool {
    !status(state_dir, root).protected
}

/// What the DJ is told when a write is refused because the Stick is protected.
#[must_use]
pub fn refusal(root: &Path) -> String {
    let name = name_of(root);
    format!(
        "{name} is protected: nothing is written to it until it is unlocked. Nothing was written. \
         Unlock it in its panel, from the command palette, or with `wreckordbox stick unlock {}`.",
        root.display()
    )
}

/// Unlocks the Stick at `root` for its current mount. A Gig stick needs
/// `typed` to be its name (surrounding spaces aside).
///
/// # Errors
///
/// See [`UnlockError`]; the Stick stays protected.
pub fn unlock(state_dir: &Path, root: &Path, typed: Option<&str>) -> Result<Status, UnlockError> {
    let mount = mount_of(root).map_err(|_| UnlockError::NotFound)?;
    let name = name_of(root);
    if !is_spare(root) {
        match typed.map(str::trim) {
            None | Some("") => return Err(UnlockError::NameRequired { name }),
            Some(typed) if typed != name => return Err(UnlockError::NameMismatch { name }),
            Some(_) => {}
        }
    }
    change(state_dir, |stored| {
        stored.unlocked.retain(|m| m.path != mount.path);
        stored.unlocked.push(mount.clone());
    })
    .map_err(|e| UnlockError::Io(e.to_string()))?;
    Ok(status(state_dir, root))
}

/// Protects the Stick at `root` again (the DJ's lock, an eject).
///
/// # Errors
///
/// When the record cannot be written. A Stick whose unlock cannot be
/// written away is still protected once it is removed or put back.
pub fn lock(state_dir: &Path, root: &Path) -> std::io::Result<Status> {
    let path = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    change(state_dir, |stored| {
        stored.unlocked.retain(|m| m.path != path)
    })?;
    Ok(status(state_dir, root))
}

/// One change of the record, under the lock, with a higher revision; mounts
/// that are no longer there are dropped as it goes.
fn change(state_dir: &Path, edit: impl FnOnce(&mut Stored)) -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let turn = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(state_dir.join(LOCK_FILE))?;
    turn.lock()?;
    let mut stored = read_stored(state_dir);
    stored
        .unlocked
        .retain(|m| mount_of(&m.path).is_ok_and(|now| now == *m));
    edit(&mut stored);
    stored.revision += 1;
    let partial = state_dir.join(format!("{FILE}.{}.partial", std::process::id()));
    let written = std::fs::File::create(&partial).and_then(|mut file| {
        file.write_all(&serde_json::to_vec(&stored).map_err(std::io::Error::other)?)?;
        file.sync_all()
    });
    if let Err(error) = written.and_then(|()| std::fs::rename(&partial, state_dir.join(FILE))) {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    drop(turn);
    Ok(())
}

/// The root of the Stick that `destination` (a folder or a file, existing
/// or not yet) is on, or `None` for an ordinary folder.
///
/// The destination is resolved first (its nearest existing folder made
/// canonical), then matched against the mounted Sticks' folders and,
/// walking up, against a folder carrying [`STAND_IN_MARKER`].
#[must_use]
pub fn stick_root_of(destination: &Path, mounted: &[PathBuf]) -> Option<PathBuf> {
    let resolved = resolve(destination)?;
    let mut best: Option<PathBuf> = None;
    for mount in mounted {
        let Ok(mount) = mount.canonicalize() else {
            continue;
        };
        if resolved.starts_with(&mount)
            && best
                .as_ref()
                .is_none_or(|b| mount.as_os_str().len() > b.as_os_str().len())
        {
            best = Some(mount);
        }
    }
    if best.is_some() {
        return best;
    }
    resolved
        .ancestors()
        .find(|dir| dir.join(STAND_IN_MARKER).is_file())
        .map(Path::to_path_buf)
}

/// `path` made absolute and canonical through its nearest existing
/// ancestor, the rest appended (with `..` and `.` resolved).
fn resolve(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut rest = Vec::new();
    let mut existing = absolute.as_path();
    loop {
        if let Ok(found) = existing.canonicalize() {
            let mut out = found;
            for part in rest.iter().rev() {
                match part {
                    std::path::Component::ParentDir => {
                        out.pop();
                    }
                    std::path::Component::Normal(name) => out.push(name),
                    _ => {}
                }
            }
            return Some(out);
        }
        let component = existing.components().next_back()?;
        rest.push(component);
        existing = existing.parent()?;
    }
}
