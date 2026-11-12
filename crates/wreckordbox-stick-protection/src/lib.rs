//! Stick Protection (wbx-45.11): nothing writes to a Stick, or settles an
//! unfinished write on it, unless the DJ has unlocked that Stick.
//!
//! The backstop under every Stick write, whoever asks (the window in either
//! layout, the Library service, the command line, a script, background
//! work). The window's staged plan and hold are a per-operation
//! confirmation in the interface on top of it; this is the guarantee.
//!
//! - **What is a Stick** ([`classify`]) is decided from the volume itself,
//!   never from a list the interface was shown: the volume (mount) holding
//!   the path carries a player export (a `PIONEER` folder or the Spare stick
//!   marker at its root), or, when its root cannot be read, it is removable
//!   or mounted under the media folders (fail closed). Any other volume,
//!   an ordinary external disk with no player export included, is not a
//!   Stick. With `RBXPORT_TEST=1` a folder carrying [`STAND_IN_MARKER`]
//!   stands in for a Stick; without it, no test override is honoured.
//! - **One Stick, one identity**: status, unlock, lock and every writer
//!   resolve a path to its containing Stick's root; a folder inside a Stick
//!   is never unlocked on its own.
//! - **Protected by default.** An unlock is recorded for one mount
//!   incarnation ([`Identity`]): the boot it happened in, the kernel's
//!   never-reused mount id, and the root's device and inode. It ends when
//!   the Stick is ejected, pulled out or put back, when the machine
//!   restarts, or when the DJ locks it. When the identity cannot be
//!   established, the Stick stays protected.
//! - **Unlocking names the Stick.** A Spare stick ([`SPARE_MARKER`])
//!   unlocks with a plain confirmation; any other Stick needs its name typed,
//!   checked here.
//! - **One execution protocol** ([`exec_lock`]): every writer holds the
//!   Stick's execution lock, an exclusive lock on a file in the state folder
//!   (never on the Stick), for its whole life, across processes. Locking a
//!   Stick and ejecting it take the same lock first, so neither is
//!   acknowledged while a write runs. A long writer re-checks
//!   ([`Authorization::recheck`]) before each step and stops when the Stick
//!   was replaced.
//! - **Kept like Library Protection**: one record, [`FILE`], in the shared
//!   state folder; changes under an exclusive lock on [`LOCK_FILE`], written
//!   to a temporary file and renamed over the old, with a rising revision. A
//!   missing, unreadable or corrupt record unlocks nothing.

use std::io::Write as _;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The record's name inside the shared state folder.
pub const FILE: &str = "stick-protection.json";
/// The file every writer of [`FILE`] holds an exclusive lock on.
pub const LOCK_FILE: &str = "stick-protection.lock";
/// The folder of the Sticks' execution locks, inside the state folder.
pub const EXEC_LOCKS: &str = "stick-locks";
/// The marker at a Stick's root that declares it a Spare stick.
pub const SPARE_MARKER: &str = ".rbxport-disposable";
/// With `RBXPORT_TEST=1`, a folder carrying this file stands in for a
/// mounted Stick. Ignored otherwise.
pub const STAND_IN_MARKER: &str = ".rbxport-stands-for-stick";
/// The stable error code of a write refused on a protected Stick.
pub const CODE: &str = "stick_protected";

/// `statx`'s `STATX_MNT_ID_UNIQUE` (Linux 6.8): a mount id never reused
/// while the machine runs.
const STATX_MNT_ID_UNIQUE: u32 = 0x4000;

/// What a path is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Volume {
    /// Not a Stick: never refused here.
    Ordinary,
    /// On the Stick whose root (mount point) is `root`.
    Stick { root: PathBuf },
}

/// One mount incarnation of a Stick: an unlock holds for exactly this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub root: PathBuf,
    /// `/proc/sys/kernel/random/boot_id`: a restart ends every unlock.
    pub boot: String,
    /// The kernel's unique mount id, never reused within a boot.
    pub mount: u64,
    pub dev: u64,
    pub ino: u64,
}

/// Whether a Stick is protected, as the DJ is shown it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub protected: bool,
    /// It carries the Spare stick marker: it unlocks with a confirmation.
    pub spare: bool,
    /// The name the DJ types to unlock a Gig stick: its root folder's.
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
    /// There is no Stick there (an ordinary folder, or nothing).
    NotAStick,
    /// This mount of the Stick cannot be told apart from another: it stays
    /// protected.
    NoIdentity,
    /// The record could not be written; the Stick stays protected.
    Io(String),
}

impl std::fmt::Display for UnlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NameRequired { name } => write!(f, "{name} is a Gig stick: type its name, {name}, to unlock it."),
            Self::NameMismatch { name } => write!(f, "That is not this Stick's name. Type {name} exactly to unlock it."),
            Self::NotAStick => f.write_str("That is not a Stick, so there is nothing to unlock."),
            Self::NoIdentity => f.write_str(
                "This Stick's mount cannot be identified, so it stays protected. Eject it and plug it in again.",
            ),
            Self::Io(why) => write!(f, "The Stick could not be unlocked, so it stays protected: {why}"),
        }
    }
}

impl std::error::Error for UnlockError {}

/// Why a write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The Stick is protected.
    Protected { root: PathBuf },
    /// The Stick was removed, or replaced by another, since the write began.
    Replaced { root: PathBuf },
    /// The Stick's execution lock could not be taken.
    Io { root: PathBuf, why: String },
}

impl Refused {
    #[must_use]
    pub fn root(&self) -> &Path {
        match self {
            Self::Protected { root } | Self::Replaced { root } | Self::Io { root, .. } => root,
        }
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protected { root } => f.write_str(&refusal(root)),
            Self::Replaced { root } => write!(
                f,
                "{} was removed or replaced while it was being written to: the write stopped there. \
                 Anything not yet written was not written.",
                name_of(root)
            ),
            Self::Io { root, why } => write!(
                f,
                "{} could not be reserved for writing, so nothing was written: {why}",
                name_of(root)
            ),
        }
    }
}

impl std::error::Error for Refused {}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    revision: u64,
    #[serde(default)]
    unlocked: Vec<Identity>,
}

/// The Stick's name: its root folder's.
#[must_use]
pub fn name_of(root: &Path) -> String {
    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Whether the Stick carries the Spare stick marker.
#[must_use]
pub fn is_spare(root: &Path) -> bool {
    root.join(SPARE_MARKER).is_file()
}

fn test_mode() -> bool {
    std::env::var_os("RBXPORT_TEST").is_some_and(|v| v == "1")
}

// ---------------------------------------------------------------------------
// What is a Stick

/// What is known about the volume (mount) holding a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeFacts {
    /// Its mount point.
    pub root: PathBuf,
    /// Removable or USB storage; `None` when it cannot be told.
    pub removable: Option<bool>,
    /// Mounted under the media folders (`/run/media`, `/media`, `/Volumes`).
    pub media: bool,
    /// A player export (or the Spare stick marker) at its root; `None` when
    /// its root cannot be read.
    pub export: Option<bool>,
}

/// The rule: a player export makes a Stick; a root that cannot be read is
/// a Stick when the volume is removable (or may be) or under the media
/// folders; anything else is not.
#[must_use]
pub fn is_stick(facts: &VolumeFacts) -> bool {
    match facts.export {
        Some(export) => export,
        None => facts.media || facts.removable != Some(false),
    }
}

/// Whether `path` is on a Stick, and which: decided from the volume.
#[must_use]
pub fn classify(path: &Path) -> Volume {
    let Some(resolved) = resolve(path) else {
        return Volume::Ordinary;
    };
    if test_mode() {
        if let Some(root) = resolved.ancestors().find(|dir| dir.join(STAND_IN_MARKER).is_file()) {
            return Volume::Stick { root: root.to_path_buf() };
        }
    }
    match facts_of(&resolved) {
        Some(facts) if is_stick(&facts) => Volume::Stick { root: facts.root },
        // An ordinary volume, or not even a mount found: nothing to protect it by.
        _ => Volume::Ordinary,
    }
}

/// The facts of the volume holding `resolved` (canonical).
fn facts_of(resolved: &Path) -> Option<VolumeFacts> {
    let mount = containing_mount(resolved)?;
    let media = ["/run/media", "/media", "/Volumes"].iter().any(|dir| mount.point.starts_with(dir));
    Some(VolumeFacts { export: player_export(&mount.point), removable: removable(&mount.device), media, root: mount.point })
}

fn player_export(root: &Path) -> Option<bool> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut found = false;
    for entry in entries {
        let name = entry.ok()?.file_name();
        let name = name.to_string_lossy();
        if name.eq_ignore_ascii_case("PIONEER") || name == SPARE_MARKER {
            found = true;
        }
    }
    Some(found)
}

struct Mount {
    point: PathBuf,
    /// `major:minor`.
    device: String,
}

/// The mount whose mount point is the longest prefix of `path`.
fn containing_mount(path: &Path) -> Option<Mount> {
    let table = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    let mut best: Option<Mount> = None;
    for line in table.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let (Some(device), Some(point)) = (fields.get(2), fields.get(4)) else { continue };
        let point = PathBuf::from(unescape(point));
        if path.starts_with(&point)
            && best.as_ref().is_none_or(|b| point.components().count() >= b.point.components().count())
        {
            best = Some(Mount { point, device: (*device).to_owned() });
        }
    }
    best
}

/// `/proc/self/mountinfo`'s octal escapes (`\040` for a space).
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' && at + 3 < bytes.len() && bytes[at + 1..at + 4].iter().all(u8::is_ascii_digit) {
            if let Some(code) = std::str::from_utf8(&bytes[at + 1..at + 4]).ok().and_then(|s| u8::from_str_radix(s, 8).ok()) {
                out.push(code);
                at += 4;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether the block device `major:minor` is removable or on USB.
fn removable(device: &str) -> Option<bool> {
    if device.starts_with("0:") {
        // No block device behind it (tmpfs, a network or virtual filesystem).
        return Some(false);
    }
    let node = Path::new("/sys/dev/block").join(device).canonicalize().ok()?;
    let usb = node.to_string_lossy().contains("/usb");
    let flag = |dir: &Path| std::fs::read_to_string(dir.join("removable")).ok().map(|text| text.trim() == "1");
    // A partition's own folder has no `removable`: its disk's says.
    match flag(&node).or_else(|| node.parent().and_then(flag)) {
        Some(removable) => Some(removable || usb),
        None if usb => Some(true),
        None => None,
    }
}

/// `path` made absolute and canonical through its nearest existing
/// ancestor, the rest appended (with `..` and `.` resolved).
fn resolve(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().ok()?.join(path) };
    let mut rest = Vec::new();
    let mut existing = absolute.as_path();
    loop {
        if let Ok(found) = existing.canonicalize() {
            let mut out = found;
            for part in rest.iter().rev() {
                match part {
                    Component::ParentDir => {
                        out.pop();
                    }
                    Component::Normal(name) => out.push(name),
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

/// The root of the Stick `path` is on, if it is on one.
#[must_use]
pub fn stick_root(path: &Path) -> Option<PathBuf> {
    match classify(path) {
        Volume::Stick { root } => Some(root),
        Volume::Ordinary => None,
    }
}

/// The root of the Stick `path` is on, or of the removable or media volume
/// it is on: a blank Stick, with no player export yet, is still a device
/// the Stick-only writes (a library put on it, a format) need unlocked.
#[must_use]
pub fn device_root(path: &Path) -> Option<PathBuf> {
    stick_root(path).or_else(|| {
        let facts = facts_of(&resolve(path)?)?;
        (facts.media || facts.removable == Some(true)).then_some(facts.root)
    })
}

// ---------------------------------------------------------------------------
// Identity

fn boot_id() -> Option<String> {
    let id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

/// This mount incarnation of the Stick at `root`; `None` when it cannot be
/// established (the Stick is then protected).
#[must_use]
pub fn identity(root: &Path) -> Option<Identity> {
    identity_in(root, boot_id())
}

fn identity_in(root: &Path, boot: Option<String>) -> Option<Identity> {
    use std::os::unix::fs::MetadataExt as _;
    let boot = boot?;
    let root = root.canonicalize().ok()?;
    let mask = rustix::fs::StatxFlags::from_bits_retain(STATX_MNT_ID_UNIQUE) | rustix::fs::StatxFlags::INO;
    let found = rustix::fs::statx(rustix::fs::CWD, &root, rustix::fs::AtFlags::empty(), mask).ok()?;
    if found.stx_mask & STATX_MNT_ID_UNIQUE == 0 {
        return None;
    }
    let meta = std::fs::metadata(&root).ok()?;
    Some(Identity { root, boot, mount: found.stx_mnt_id, dev: meta.dev(), ino: meta.ino() })
}

// ---------------------------------------------------------------------------
// The record

fn read_stored(state_dir: &Path) -> Stored {
    std::fs::read(state_dir.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

/// Whether the Stick holding `path` is protected now. A path on no Stick
/// is reported as its own folder, protected (there is nothing to unlock).
#[must_use]
pub fn status(state_dir: &Path, path: &Path) -> Status {
    let stored = read_stored(state_dir);
    let root = device_root(path).unwrap_or_else(|| path.to_path_buf());
    let unlocked = identity(&root).is_some_and(|now| stored.unlocked.contains(&now));
    Status { protected: !unlocked, spare: is_spare(&root), name: name_of(&root), revision: stored.revision }
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

/// Unlocks the Stick holding `path` for its current mount. A Gig stick needs
/// `typed` to be its name (surrounding spaces aside).
///
/// # Errors
///
/// See [`UnlockError`]; the Stick stays protected.
pub fn unlock(state_dir: &Path, path: &Path, typed: Option<&str>) -> Result<Status, UnlockError> {
    let root = device_root(path).ok_or(UnlockError::NotAStick)?;
    let name = name_of(&root);
    if !is_spare(&root) {
        match typed.map(str::trim) {
            None | Some("") => return Err(UnlockError::NameRequired { name }),
            Some(typed) if typed != name => return Err(UnlockError::NameMismatch { name }),
            Some(_) => {}
        }
    }
    let now = identity(&root).ok_or(UnlockError::NoIdentity)?;
    change(state_dir, |stored| {
        stored.unlocked.retain(|m| m.root != now.root);
        stored.unlocked.push(now.clone());
    })
    .map_err(|e| UnlockError::Io(e.to_string()))?;
    Ok(status(state_dir, &root))
}

/// Protects the Stick holding `path` again (the DJ's lock, an eject). Waits
/// for any write to it to finish first (its execution lock), so a lock is
/// never acknowledged while something is still writing.
///
/// # Errors
///
/// When the record cannot be written. A Stick whose unlock cannot be
/// written away is still protected once it is removed or put back.
pub fn lock(state_dir: &Path, path: &Path) -> std::io::Result<Status> {
    let root = root_of(path);
    let drained = exec_lock(state_dir, &root)?;
    lock_held(state_dir, &root, &drained)
}

/// The root [`lock`] and [`exec_lock`] callers use for `path`: its Stick's
/// (or removable volume's), else the folder itself.
#[must_use]
pub fn root_of(path: &Path) -> PathBuf {
    device_root(path).unwrap_or_else(|| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
}

/// [`lock`] by a caller already holding the Stick's execution lock (an
/// eject, which keeps it until the volume is gone).
///
/// # Errors
///
/// When the record cannot be written.
pub fn lock_held(state_dir: &Path, root: &Path, _held: &ExecLock) -> std::io::Result<Status> {
    change(state_dir, |stored| stored.unlocked.retain(|m| m.root != root))?;
    Ok(status(state_dir, root))
}

/// One change of the record, under the lock, with a higher revision;
/// unlocks of mounts that are no longer there are dropped as it goes.
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
    stored.unlocked.retain(|m| identity(&m.root).is_some_and(|now| now == *m));
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

// ---------------------------------------------------------------------------
// The execution protocol

/// The Stick's execution lock: an exclusive lock on a file in the state
/// folder, held across processes and threads. Re-entrant for the thread
/// holding it (a writer whose own write reaches another guarded step, a
/// Library edit that recovers, never waits on itself). Released when the
/// last hold of that thread is dropped.
#[derive(Debug)]
pub struct ExecLock {
    key: PathBuf,
}

/// This process's holds: the lock file's path → the thread holding it, how
/// many times, and the locked file (`None` while it is being taken).
struct Hold {
    thread: std::thread::ThreadId,
    count: usize,
    file: Option<std::fs::File>,
}

type Holds = std::collections::HashMap<PathBuf, Hold>;

static HOLDS: std::sync::LazyLock<(std::sync::Mutex<Holds>, std::sync::Condvar)> =
    std::sync::LazyLock::new(Default::default);

fn holds() -> std::sync::MutexGuard<'static, Holds> {
    HOLDS.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Drop for ExecLock {
    fn drop(&mut self) {
        let mut held = holds();
        if let Some(hold) = held.get_mut(&self.key) {
            hold.count -= 1;
            if hold.count == 0 {
                // Dropping the file releases the lock for other processes.
                held.remove(&self.key);
                HOLDS.1.notify_all();
            }
        }
    }
}

fn exec_path(state_dir: &Path, root: &Path) -> std::io::Result<PathBuf> {
    use std::hash::{Hash as _, Hasher as _};
    let dir = state_dir.join(EXEC_LOCKS);
    std::fs::create_dir_all(&dir)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.as_os_str().hash(&mut hasher);
    Ok(dir.join(format!("{:016x}.lock", hasher.finish())))
}

fn open_exec(key: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(key)
}

/// The thread's own hold taken again, or `false` when it holds none.
fn again(held: &mut Holds, key: &Path) -> bool {
    let me = std::thread::current().id();
    match held.get_mut(key) {
        Some(hold) if hold.thread == me => {
            hold.count += 1;
            true
        }
        _ => false,
    }
}

/// Takes the execution lock of the Stick whose root is `root`, waiting for
/// whoever holds it (another thread, another process).
///
/// # Errors
///
/// When the lock file cannot be opened.
pub fn exec_lock(state_dir: &Path, root: &Path) -> std::io::Result<ExecLock> {
    let key = exec_path(state_dir, root)?;
    let mut held = holds();
    loop {
        if again(&mut held, &key) {
            return Ok(ExecLock { key });
        }
        if !held.contains_key(&key) {
            break;
        }
        held = HOLDS.1.wait(held).unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    let me = std::thread::current().id();
    held.insert(key.clone(), Hold { thread: me, count: 1, file: None });
    drop(held);
    // Across processes: waits here, holding nothing but this process's turn.
    let taken = open_exec(&key).and_then(|file| file.lock().map(|()| file));
    let mut held = holds();
    match taken {
        Ok(file) => {
            if let Some(hold) = held.get_mut(&key) {
                hold.file = Some(file);
            }
            Ok(ExecLock { key })
        }
        Err(error) => {
            held.remove(&key);
            HOLDS.1.notify_all();
            Err(error)
        }
    }
}

/// [`exec_lock`] without waiting: `None` while another thread or process
/// holds it.
///
/// # Errors
///
/// When the lock file cannot be opened.
pub fn try_exec_lock(state_dir: &Path, root: &Path) -> std::io::Result<Option<ExecLock>> {
    let key = exec_path(state_dir, root)?;
    let mut held = holds();
    if again(&mut held, &key) {
        return Ok(Some(ExecLock { key }));
    }
    if held.contains_key(&key) {
        return Ok(None);
    }
    let file = open_exec(&key)?;
    match file.try_lock() {
        Ok(()) => {
            let me = std::thread::current().id();
            held.insert(key.clone(), Hold { thread: me, count: 1, file: Some(file) });
            Ok(Some(ExecLock { key }))
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

/// Leave to write to `path`: nothing to hold for an ordinary folder; for a
/// Stick, its execution lock and the identity it was unlocked for, held for
/// as long as this lives.
#[derive(Debug)]
pub struct Authorization {
    stick: Option<(Identity, ExecLock)>,
    state_dir: PathBuf,
}

impl Authorization {
    /// The Stick written to, or `None` for an ordinary folder.
    #[must_use]
    pub fn stick(&self) -> Option<&Path> {
        self.stick.as_ref().map(|(identity, _)| identity.root.as_path())
    }

    /// Ends the unlock of the Stick this holds, as [`lock`] does, without
    /// taking its execution lock again (this already holds it): a write
    /// that ejects the Stick when it is done.
    ///
    /// # Errors
    ///
    /// When the record cannot be written.
    pub fn relock(&self) -> std::io::Result<()> {
        let Some((held, _)) = &self.stick else { return Ok(()) };
        change(&self.state_dir, |stored| stored.unlocked.retain(|m| m.root != held.root))
    }

    /// Whether the write may go on: the same mount of the Stick is still
    /// there, and still unlocked. Call before each step of a long write.
    ///
    /// # Errors
    ///
    /// [`Refused::Replaced`] or [`Refused::Protected`].
    pub fn recheck(&self) -> Result<(), Refused> {
        let Some((held, _)) = &self.stick else { return Ok(()) };
        if identity(&held.root).as_ref() != Some(held) {
            return Err(Refused::Replaced { root: held.root.clone() });
        }
        if !read_stored(&self.state_dir).unlocked.contains(held) {
            return Err(Refused::Protected { root: held.root.clone() });
        }
        Ok(())
    }
}

/// Authorizes a write to `path`: classified from its volume; on a Stick, the
/// execution lock is taken (waiting for another writer, or a lock or eject
/// in progress), then the Stick must be unlocked for this very mount.
///
/// # Errors
///
/// [`Refused`]: nothing may be written.
pub fn authorize(state_dir: &Path, path: &Path) -> Result<Authorization, Refused> {
    match classify(path) {
        Volume::Ordinary => Ok(Authorization { stick: None, state_dir: state_dir.to_path_buf() }),
        Volume::Stick { root } => authorize_stick(state_dir, &root),
    }
}

/// [`authorize`] for a path that is a Stick whatever its volume says (a
/// Stick's own playlists, settings, format): its root is `root`.
///
/// # Errors
///
/// [`Refused`]: nothing may be written.
pub fn authorize_stick(state_dir: &Path, root: &Path) -> Result<Authorization, Refused> {
    let root = device_root(root).unwrap_or_else(|| root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
    let held = exec_lock(state_dir, &root).map_err(|e| Refused::Io { root: root.clone(), why: e.to_string() })?;
    let Some(now) = identity(&root) else {
        return Err(Refused::Protected { root });
    };
    if !read_stored(state_dir).unlocked.contains(&now) {
        return Err(Refused::Protected { root });
    }
    Ok(Authorization { stick: Some((now, held)), state_dir: state_dir.to_path_buf() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(export: Option<bool>, removable: Option<bool>, media: bool) -> VolumeFacts {
        VolumeFacts { root: PathBuf::from("/x"), removable, media, export }
    }

    #[test]
    fn the_rule_reads_the_volume_and_fails_closed_on_removable_media() {
        assert!(is_stick(&facts(Some(true), Some(false), false)), "a player export, even on a fixed disk");
        assert!(!is_stick(&facts(Some(false), Some(true), true)), "an external disk with no player export");
        assert!(is_stick(&facts(None, Some(true), false)), "unreadable and removable");
        assert!(is_stick(&facts(None, Some(false), true)), "unreadable and under the media folders");
        assert!(is_stick(&facts(None, None, false)), "unreadable and may be removable");
        assert!(!is_stick(&facts(None, Some(false), false)), "unreadable fixed disk outside the media folders");
    }

    #[test]
    fn without_a_boot_id_a_mount_has_no_identity() {
        let dir = tempfile::tempdir().unwrap();
        assert!(identity_in(dir.path(), None).is_none());
        assert!(identity_in(dir.path(), Some("b".into())).is_some(), "statx's unique mount id is available here");
    }

    #[test]
    fn mount_points_with_spaces_are_read() {
        assert_eq!(unescape("/run/media/go/DJ\\040STICK"), "/run/media/go/DJ STICK");
    }

    #[test]
    fn the_root_filesystem_is_not_a_stick() {
        assert_eq!(classify(Path::new("/usr/share")), Volume::Ordinary);
    }
}
