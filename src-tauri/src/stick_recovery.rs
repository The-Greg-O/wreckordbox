//! An unfinished write on a Stick: reported by every read, settled only when
//! the DJ asks (wbx-45.5).
//!
//! Every command that only reads a Stick reads it through
//! [`wreckordbox_stick_outline::StickRead`], whose parsers cannot settle or
//! write anything, and answers an interrupted publication with
//! [`unfinished_write`]'s error. Round-trip (`import_usb`) refuses such a
//! Stick the same way. [`stick_settle`] is the one command that settles it,
//! for the action the DJ chooses in the Stick's panel; Stick writes the DJ
//! asked for (an edit, an export, a sync, saving the device settings) still
//! settle it as their first step. `stick-recovery.md` in the notes lists
//! every path.

use std::path::{Path, PathBuf};

use wreckordbox_stick_outline::ReadError;

use crate::commands::blocking;
use crate::error::{AppError, AppResult, ErrorKind};

/// The Stick's name as the window shows it: its mount point's last part.
fn stick_name(mount: &Path) -> String {
    mount.file_name().map_or_else(|| mount.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// What a read answers for a Stick whose last write did not finish: it
/// names the action that settles it (`unfinishedWriteWording` in the window).
pub fn unfinished_write(mount: &Path) -> AppError {
    AppError::new(
        ErrorKind::UnfinishedWrite,
        format!(
            "A write to this Stick did not finish, and nothing on it has been changed. To use it, open the Stick and choose \"Finish or roll back the unfinished write on {}\".",
            stick_name(mount)
        ),
    )
}

/// A failed Stick read, for the window.
pub fn read_error(mount: &Path, error: ReadError) -> AppError {
    match error {
        ReadError::UnfinishedWrite => unfinished_write(mount),
        ReadError::Moved => AppError::new(ErrorKind::NotFound, "The Stick changed while it was being read. Try again."),
        ReadError::Unreadable(message) => AppError::new(ErrorKind::Malformed, message),
        ReadError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
            AppError::new(ErrorKind::NotFound, "That device is no longer connected.").with_detail(e.to_string())
        }
        ReadError::Io(e) => AppError::new(ErrorKind::Internal, "The Stick could not be read.").with_detail(e.to_string()),
    }
}

/// Settles the unfinished write on the Stick mounted at `path`: finishes the
/// interrupted export when it can, otherwise puts back the files it held
/// before; files another program wrote since are kept. Only for the action
/// the DJ chose in the Stick's panel.
#[tauri::command]
pub async fn stick_settle(path: String) -> AppResult<()> {
    blocking("stick_settle", move || settle(&PathBuf::from(path), rbl_db::is_rekordbox_app_running())).await
}

fn settle(mount: &Path, rekordbox_running: bool) -> AppResult<()> {
    if !mount.is_dir() {
        return Err(AppError::new(ErrorKind::NotFound, "That device is no longer connected."));
    }
    if rekordbox_running {
        return Err(AppError::new(ErrorKind::ReadOnly, "Quit rekordbox before settling this Stick so only one application writes it."));
    }
    wreckordbox_stick_outline::settle(mount).map_err(|error| {
        AppError::new(ErrorKind::Malformed, "The unfinished write on this Stick could not be settled.").with_detail(error.to_string())
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use tauri::Manager;
    use wreckordbox_stick_outline::testing::{interrupt, snapshot, stick, without_lock, JOURNAL, LOCK, NEXT_PLAYLISTS, ONE, PDB};

    use super::*;
    use crate::dto::{TrackFilterDto, TrackSourceDto, ViewSpecDto};
    use crate::state::AppState;

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    /// A window's state over a fixture Library (Library Protection off), as
    /// the import and the collection lookups need one.
    fn app(dir: &Path) -> tauri::App<tauri::test::MockRuntime> {
        let location = rbl_db::fixture::build(dir, rbl_db::fixture::Shape::default()).unwrap();
        let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let (library, _) = rbl_index::load(&db).unwrap();
        let state = AppState::unprotected(dir.join("backups"));
        state.set_library(library, false, db.schema().db_version, 0, location);
        drop(db);
        let app = tauri::test::mock_app();
        app.manage(Arc::new(state));
        app
    }

    fn device_spec(path: &Path, format: &str, playlist: &str) -> ViewSpecDto {
        ViewSpecDto {
            source: TrackSourceDto::Device { path: path.to_string_lossy().into_owned(), format: format.into(), playlist: playlist.into() },
            sort: "trackNo".into(),
            descending: false,
            query: String::new(),
            search_field: rbl_index::SearchField::All,
            filter: TrackFilterDto::default(),
        }
    }

    fn refused<T: std::fmt::Debug>(what: &str, result: AppResult<T>) {
        match result {
            Err(error) => {
                assert_eq!(error.kind, ErrorKind::UnfinishedWrite, "{what}: {error:?}");
                assert!(error.message.contains("Finish or roll back the unfinished write on"), "{what}: {}", error.message);
            }
            Ok(found) => panic!("{what}: read a Stick with an unfinished write: {found:?}"),
        }
    }

    /// Every way the window reads a Stick, in either layout, on a Stick
    /// whose last write did not finish, with and without our lock file: each
    /// reports it, and the Stick's bytes are compared after every one.
    #[test]
    fn no_read_of_a_stick_settles_or_writes_anything() {
        for lock in [true, false] {
            let (dest, dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let mount = dest.path();
            stick(mount);
            interrupt(mount);
            if !lock {
                without_lock(mount);
            }
            let before = snapshot(mount);
            let path = mount.to_string_lossy().into_owned();
            let app = app(dir.path());
            let state = app.state::<Arc<AppState>>();
            let unchanged = |what: &str| assert_eq!(snapshot(mount), before, "lock {lock}: {what} changed the Stick");

            // The old tree: expanding a Stick, and every later listing refresh.
            refused("device_libraries", run(crate::device_library::device_libraries(path.clone())));
            unchanged("device_libraries");
            refused("device_libraries again", run(crate::device_library::device_libraries(path.clone())));
            unchanged("device_libraries again");
            for format in ["deviceLibrary", "oneLibrary"] {
                // Opening a Stick playlist's tracks, and a library's All Tracks.
                for playlist in ["1", "0"] {
                    refused("open_view", run(crate::commands::open_view(state.clone(), device_spec(mount, format, playlist))));
                    unchanged("open_view");
                }
                refused("duplicates", run(crate::device_library::device_playlist_duplicates(path.clone(), format.into(), "1".into(), vec![])));
                unchanged("device_playlist_duplicates");
                refused(
                    "collection tracks",
                    run(crate::device_library::device_playlist_collection_tracks(state.clone(), path.clone(), format.into(), vec!["1".into()])),
                );
                unchanged("device_playlist_collection_tracks");
                refused("import plan", run(crate::device_library::device_playlist_import_plan(state.clone(), path.clone(), format.into(), "1".into(), None)));
                unchanged("device_playlist_import_plan");
            }
            // The Stick's panel, in either layout.
            refused("device_settings", run(crate::device_settings::device_settings(path.clone())));
            unchanged("device_settings");
            // The shell's tree.
            assert!(run(crate::stick_outline::stick_outline(path)).unwrap().pending);
            unchanged("stick_outline");
            // Round-trip: cues, history, settings and ratings.
            let editor = crate::grid::GridEditor::at(state.backup_dir());
            for (cues, history, settings, ratings) in [(true, false, false, false), (false, true, false, false), (false, false, true, false), (false, false, false, true)] {
                refused("import_usb", crate::usb_import::import(&state, &editor, mount, cues, history, settings, ratings));
                unchanged("import_usb");
            }
            assert_eq!(mount.join(LOCK).exists(), lock, "no lock file was made");
            assert!(mount.join(JOURNAL).exists());
        }
    }

    /// Library Protection is said first: the DJ is not sent to settle a
    /// Stick for an import that would be refused anyway.
    #[test]
    fn round_trip_says_library_protection_first_and_touches_nothing() {
        let (dest, dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        stick(dest.path());
        interrupt(dest.path());
        without_lock(dest.path());
        let before = snapshot(dest.path());
        let app = app(dir.path());
        let state = app.state::<Arc<AppState>>();
        state.core().set_protection(true).unwrap();
        let editor = crate::grid::GridEditor::at(state.backup_dir());
        let refused = crate::usb_import::import(&state, &editor, dest.path(), true, true, false, true).unwrap_err();
        assert_eq!(refused.message, rbl_library::protection::MESSAGE);
        assert_eq!(snapshot(dest.path()), before);
    }

    /// Opening the Stick's panel on a settled Stick that holds an export but
    /// no DEVSETTING.DAT reads it and writes nothing (no defaults).
    #[test]
    fn opening_the_panel_writes_no_defaults_or_anything_else() {
        for lock in [true, false] {
            let dest = tempfile::tempdir().unwrap();
            stick(dest.path());
            let _ = std::fs::remove_file(dest.path().join("PIONEER/DEVSETTING.DAT"));
            if !lock {
                without_lock(dest.path());
            }
            let before = snapshot(dest.path());
            let settings = run(crate::device_settings::device_settings(dest.path().to_string_lossy().into_owned())).unwrap();
            assert!(settings.has_device_library);
            assert!(!settings.has_dev_setting);
            assert_eq!(snapshot(dest.path()), before);
        }
    }

    #[test]
    fn a_settled_stick_reads_as_before_and_nothing_is_written() {
        for lock in [true, false] {
            let (dest, dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let mount = dest.path();
            stick(mount);
            if !lock {
                without_lock(mount);
            }
            let before = snapshot(mount);
            let path = mount.to_string_lossy().into_owned();
            let libraries = run(crate::device_library::device_libraries(path.clone())).unwrap();
            assert_eq!(snapshot(mount), before);
            assert_eq!(libraries.len(), 2);
            assert_eq!(libraries[0].nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["Sets", "Friday", "Warm Up"]);
            let friday = libraries[0].nodes.iter().find(|n| n.name == "Friday").unwrap().id.clone();
            let app = app(dir.path());
            let handle = run(crate::commands::open_view(app.state::<Arc<AppState>>(), device_spec(mount, "deviceLibrary", &friday))).unwrap();
            assert_eq!(handle.len, 2);
            assert_eq!(snapshot(mount), before);
            assert!(run(crate::device_settings::device_settings(path)).unwrap().has_device_library);
            assert_eq!(snapshot(mount), before);
        }
    }

    #[test]
    fn the_settle_action_publishes_exactly_the_interrupted_export() {
        for lock in [true, false] {
            let dest = tempfile::tempdir().unwrap();
            let mount = dest.path();
            stick(mount);
            let next = interrupt(mount);
            if !lock {
                without_lock(mount);
            }

            run(stick_settle(mount.to_string_lossy().into_owned())).unwrap();

            assert_eq!(std::fs::read(mount.join(PDB)).unwrap(), next.pdb);
            assert_eq!(std::fs::read(mount.join(ONE)).unwrap(), next.one);
            assert!(!mount.join(JOURNAL).exists());
            let libraries = run(crate::device_library::device_libraries(mount.to_string_lossy().into_owned())).unwrap();
            for library in &libraries {
                assert_eq!(library.nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), NEXT_PLAYLISTS, "{}", library.format);
            }
            assert_eq!(libraries.len(), 2);
        }
    }

    #[test]
    fn settling_is_refused_while_rekordbox_runs_and_changes_nothing() {
        let dest = tempfile::tempdir().unwrap();
        stick(dest.path());
        interrupt(dest.path());
        let before = snapshot(dest.path());
        let refused = settle(dest.path(), true).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::ReadOnly);
        assert_eq!(snapshot(dest.path()), before);
    }
}
