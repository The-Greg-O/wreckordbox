//! Library Protection, as the service's clients see it: the DJ's switch
//! that refuses every change to the Library until it is turned off. On by
//! default.
//!
//! The switch and its rule belong to the Library core
//! ([`rbl_library::protection`]): it is kept in a file in the folder the
//! window and the service share, and the core refuses every write while it
//! is on, under the gate that serialises edits, whoever asks. The service
//! only answers `protection.get` and `protection.set` (which turns it
//! through the core) and maps the core's refusal to [`LIBRARY_PROTECTED`].
//! Reading is never refused, and nor is writing to the Sidecar, which is
//! not the Library.

use serde::{Deserialize, Serialize};

use crate::protocol::ServiceError;

pub use rbl_library::protection::{file, read, store, FILE, MESSAGE};

/// The error code of a change refused because Library Protection is on.
pub use crate::protocol::codes::LIBRARY_PROTECTED;

/// The event a service sends its subscribed clients whenever the switch
/// is turned, by whoever; its data is a [`Protection`]. Sent in order; one
/// with a lower `revision` than one already heard is stale.
pub const PROTECTION_CHANGED: &str = "protection_changed";

/// Command names.
pub mod commands {
    /// No arguments; answers a [`Protection`](super::Protection).
    pub const GET: &str = "protection.get";
    /// A [`SetProtection`](super::SetProtection); answers a
    /// [`ProtectionSet`](super::ProtectionSet).
    pub const SET: &str = "protection.set";
}

/// Whether Library Protection is on, and the revision of the change that
/// made it so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protection {
    pub protect_library: bool,
    #[serde(default)]
    pub revision: u64,
}

impl From<rbl_library::protection::ProtectionState> for Protection {
    fn from(state: rbl_library::protection::ProtectionState) -> Self {
        Self {
            protect_library: state.protect_library,
            revision: state.revision,
        }
    }
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
    /// The revision of this change.
    #[serde(default)]
    pub revision: u64,
}

/// The refusal of a change while protection is on.
#[must_use]
pub fn refusal() -> ServiceError {
    ServiceError::new(LIBRARY_PROTECTED, MESSAGE)
}

/// Whether Library Protection is on in `state_dir`: on unless the file
/// says off.
#[must_use]
pub fn is_on(state_dir: &std::path::Path) -> bool {
    read(state_dir).protect_library
}
