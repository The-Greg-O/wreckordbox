//! Waiting until a sandbox's Library service is really gone (wbx-377).
//!
//! A service that was stopped or made to die stops answering before it
//! lets go of the Library: it answers `stop` first, and one that aborts can
//! refuse connections for a while (writing its core dump, its threads still
//! exiting) while it still holds the Library's lock. The next service
//! cannot start until that lock is free, so a test that restarts the
//! service waits on the condition the service exposes for exactly this,
//! [`wreckordbox_service::library_released`], not on a refused connection.

use std::time::{Duration, Instant};

use super::Sandbox;

/// Waits until no service holds the sandbox's Library; `what` names the
/// service in the failure.
pub fn wait_until_released(sandbox: &Sandbox, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !wreckordbox_service::library_released(sandbox.master_db()) {
        assert!(Instant::now() < deadline, "{what} still holds the Library");
        std::thread::sleep(Duration::from_millis(20));
    }
}
