//! Library Protection: the DJ's switch that refuses every change to the
//! Library until it is turned off. On by default.
//!
//! The window keeps its preferences in its own browser storage, which
//! nothing outside the window can read, so the switch has a second home: a
//! small file in the folder the window and the service already share
//! ([`Config::recovery_dir`](crate::Config)). The window writes it whenever
//! the switch changes and takes it up when it starts; the service reads it
//! before every edit. Either side may turn it, and both see the other's.
//!
//! While the file is missing or unreadable protection is **on**: a service
//! never takes silence for leave to write.
//!
//! The rule is enforced in one place, `OpenLibrary::change`, which is the
//! service's only way to the Library core's edit path. Reading is never
//! refused, and nor is writing to the Sidecar, which is not the Library.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::protocol::ServiceError;

/// The error code of a change refused because Library Protection is on.
pub use crate::protocol::codes::LIBRARY_PROTECTED;

/// What the DJ is told: the window's own words for the same refusal
/// (`refusal(true)` in `src/lib/menu.ts`), which the scripting bridge uses
/// too.
pub const MESSAGE: &str =
    "Editing is locked by Library Protection. Turn it off in Preferences to edit.";

/// The file's name inside the shared state folder.
pub const FILE: &str = "library-protection.json";

/// The event a service sends its host and its subscribed clients when a
/// client turns the switch; its data is a [`Protection`].
pub const PROTECTION_CHANGED: &str = "protection_changed";

/// Command names.
pub mod commands {
    /// No arguments; answers a [`Protection`](super::Protection).
    pub const GET: &str = "protection.get";
    /// A [`SetProtection`](super::SetProtection); answers a
    /// [`ProtectionSet`](super::ProtectionSet).
    pub const SET: &str = "protection.set";
}

/// Whether Library Protection is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protection {
    pub protect_library: bool,
}

/// Turn Library Protection on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetProtection {
    pub protect_library: bool,
}

/// What turning the switch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectionSet {
    pub protect_library: bool,
    /// What it was before.
    pub previous: bool,
}

/// Where the switch is kept, given the shared state folder.
#[must_use]
pub fn file(state_dir: &Path) -> PathBuf {
    state_dir.join(FILE)
}

/// What the file says, or `None` when nothing readable has been recorded.
#[must_use]
pub fn stored(state_dir: &Path) -> Option<bool> {
    let text = std::fs::read(file(state_dir)).ok()?;
    serde_json::from_slice::<Protection>(&text)
        .ok()
        .map(|protection| protection.protect_library)
}

/// Whether a change to the Library is to be refused now: on unless the file
/// says off.
#[must_use]
pub fn is_on(state_dir: &Path) -> bool {
    stored(state_dir) != Some(false)
}

/// Records the switch, replacing the file whole so a reader never sees half
/// of one.
pub fn store(state_dir: &Path, protect_library: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let path = file(state_dir);
    let partial = state_dir.join(format!("{FILE}.{}.tmp", std::process::id()));
    let text = serde_json::to_vec(&Protection { protect_library })?;
    let written = std::fs::File::create(&partial).and_then(|mut out| {
        out.write_all(&text)?;
        out.sync_all()
    });
    if let Err(error) = written.and_then(|()| std::fs::rename(&partial, &path)) {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    Ok(())
}

/// The refusal of a change while protection is on.
#[must_use]
pub fn refusal() -> ServiceError {
    ServiceError::new(LIBRARY_PROTECTED, MESSAGE)
}
