//! The service keeps its recovery journals where the window keeps them, and
//! that folder is wreckordbox's own, not an installed rbxport's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// One test in its own process, because it changes the environment.
#[cfg(target_os = "linux")]
#[test]
fn the_recovery_folder_defaults_to_wreckordboxs_backup_state() {
    let data = std::path::Path::new("/nonexistent/wreckordbox-identity-test/data");
    std::env::set_var("XDG_DATA_HOME", data);
    std::env::set_var(
        wreckordbox_service::RUNTIME_DIR_ENV,
        "/nonexistent/wreckordbox-identity-test/run",
    );
    std::env::remove_var(rbl_backup::STATE_DIR_ENV);

    let config = wreckordbox_service::Config::from_env().unwrap();

    assert_eq!(config.recovery_dir, data.join("wreckordbox/backups"));
}
