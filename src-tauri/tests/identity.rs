//! Wreckordbox's own identity: its name, its application identifier and the
//! folders it keeps its files in, so it can sit beside an installed rbxport
//! without either touching the other's settings, state, logs or caches.
//!
//! The framework derives the settings, webview-data and cache folders from
//! the identifier; the app shell derives the log, backup-state and grid-lock
//! folders from a folder name under the user's data directory. Crate and
//! package names are not part of the identity (ADR 0003) and stay as they
//! are.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::Value;

fn config() -> Value {
    serde_json::from_str(include_str!("../tauri.conf.json")).unwrap()
}

#[test]
fn the_product_is_named_wreckordbox() {
    assert_eq!(config()["productName"], "wreckordbox");
}

#[test]
fn the_application_identifier_is_wreckordboxs_own() {
    assert_eq!(config()["identifier"], "com.wreckordbox.app");
}

/// The desktop app's binary is what a launcher entry runs and what the
/// compositor reports as the window's class. A `wreckordbox` binary already
/// exists (the CLI), so the desktop one has its own name, and the package
/// keeps the name upstream gave it.
#[test]
fn the_desktop_binary_has_its_own_name_and_the_package_keeps_its_name() {
    assert_eq!(config()["mainBinaryName"], "wreckordbox-desktop");
    assert!(std::path::Path::new(env!("CARGO_BIN_EXE_wreckordbox-desktop")).is_file());
    assert_eq!(env!("CARGO_PKG_NAME"), "rbxport");
}

/// One test, because it changes the process environment: with nothing
/// overriding them, every folder the app shell derives for itself is under
/// `wreckordbox/` in the user's data directory.
#[cfg(target_os = "linux")]
#[test]
fn logs_backup_state_and_grid_locks_live_under_wreckordboxs_data_folder() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("data");
    std::env::set_var("XDG_DATA_HOME", &data);
    std::env::remove_var(rbxport_lib::logging::LOG_DIR_ENV);
    std::env::remove_var(rbl_backup::STATE_DIR_ENV);

    assert_eq!(rbxport_lib::logging::log_dir(), data.join("wreckordbox/logs"));
    assert_eq!(rbl_backup::state_dir(), data.join("wreckordbox/backups"));

    // Locking a track's grid is the first thing the grid editor writes.
    rbxport_lib::grid::GridEditor::new().set_locked("1", true).unwrap();
    assert!(data.join("wreckordbox/grid-locks.json").is_file());

    let made: Vec<_> = std::fs::read_dir(&data).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(made, ["wreckordbox"], "only wreckordbox's folder is made under the data directory");
}
