//! An unfinished write on a Stick: reported by every read, settled only when
//! the DJ asks (wbx-45.5).
//!
//! Every command that only reads a Stick (`device_libraries`, a Stick view's
//! `open_view`, the duplicate and collection lookups, Import Playlist's plan,
//! the device panel's settings, `stick_outline`) reads it through
//! [`wreckordbox_stick_outline::library`] or checks
//! [`wreckordbox_stick_outline::unfinished_write`] first, and answers an
//! interrupted publication with [`unfinished_write`]'s error instead of
//! settling it. [`stick_settle`] is the one command that settles it, for the
//! action the DJ chooses; writes the DJ asked for (an edit, an export, a
//! sync, saving the device settings) still settle it as their first step.

use std::path::PathBuf;

use crate::commands::blocking;
use crate::error::{AppError, AppResult, ErrorKind};

/// What a read answers for a Stick whose last write did not finish.
pub fn unfinished_write() -> AppError {
    AppError::new(
        ErrorKind::UnfinishedWrite,
        "A write to this Stick did not finish. Nothing on it has been changed; settle it from the Stick's panel to use it again.",
    )
}

/// Settles the unfinished write on the Stick mounted at `path`: finishes the
/// interrupted export when it can, otherwise puts back the files it held
/// before; files another program wrote since are kept. Only for the action
/// the DJ chose in the Stick's panel.
#[tauri::command]
pub async fn stick_settle(path: String) -> AppResult<()> {
    blocking("stick_settle", move || {
        settle(&PathBuf::from(path), rbl_db::is_rekordbox_app_running())
    })
    .await
}

fn settle(mount: &std::path::Path, rekordbox_running: bool) -> AppResult<()> {
    if !mount.is_dir() {
        return Err(AppError::new(
            ErrorKind::NotFound,
            "That device is no longer connected.",
        ));
    }
    if rekordbox_running {
        return Err(AppError::new(
            ErrorKind::ReadOnly,
            "Quit rekordbox before settling this Stick so only one application writes it.",
        ));
    }
    wreckordbox_stick_outline::settle(mount).map_err(|error| {
        AppError::new(
            ErrorKind::Malformed,
            "The unfinished write on this Stick could not be settled.",
        )
        .with_detail(error.to_string())
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use tauri::Manager;

    use super::*;
    use crate::dto::{TrackFilterDto, TrackSourceDto, ViewSpecDto};
    use crate::state::AppState;

    fn run<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    /// A Stick with a folder holding one list, and a list at the top level,
    /// written by the export pipeline into a temporary folder.
    fn stick() -> (tempfile::TempDir, tempfile::TempDir) {
        let (src, dest) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let tracks: Vec<rbl_export::SourceTrack> = (1..=3_u8)
            .map(|i| {
                let path = src.path().join(format!("t{i}.mp3"));
                std::fs::write(&path, vec![i; 512]).unwrap();
                rbl_export::SourceTrack {
                    id: u64::from(i),
                    source_path: path,
                    title: format!("Track {i}"),
                    artist: "TRIODE".into(),
                    ..Default::default()
                }
            })
            .collect();
        let playlists = vec![
            rbl_export::SourcePlaylist {
                id: 10,
                name: "Sets".into(),
                folder: true,
                ..Default::default()
            },
            rbl_export::SourcePlaylist {
                id: 11,
                name: "Friday".into(),
                parent_id: 10,
                track_indices: vec![0, 1],
                ..Default::default()
            },
            rbl_export::SourcePlaylist {
                id: 12,
                name: "Warm Up".into(),
                track_indices: vec![2],
                ..Default::default()
            },
        ];
        rbl_export::export(dest.path(), &tracks, &playlists).unwrap();
        (src, dest)
    }

    /// What an export that stopped part way leaves: `export.pdb` already
    /// replaced, the Stick's own copy set aside in the journal, the new
    /// database still an image there. Recovery would rewrite the Stick.
    fn interrupt(root: &Path) {
        let pdb = "PIONEER/rekordbox/export.pdb";
        let db = "PIONEER/rekordbox/exportLibrary.db";
        let before = std::fs::read(root.join(pdb)).unwrap();
        std::fs::write(root.join(pdb), b"the new export.pdb").unwrap();
        let journal = root.join(".rbxport-publication");
        let image = journal.join(db);
        std::fs::create_dir_all(image.parent().unwrap()).unwrap();
        std::fs::write(&image, b"the new database").unwrap();
        let aside = journal.join(".previous").join(pdb);
        std::fs::create_dir_all(aside.parent().unwrap()).unwrap();
        std::fs::write(&aside, before).unwrap();
        std::fs::write(
            journal.join("publication.json"),
            format!(r#"[{{"path":"{pdb}","present":true,"had_target":true,"exact":true}},{{"path":"{db}","present":true,"had_target":true,"exact":true}}]"#),
        )
        .unwrap();
    }

    /// Every file under `root` and its bytes, the directories too.
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                if path.is_dir() {
                    out.insert(relative, None);
                    stack.push(path);
                } else {
                    out.insert(relative, Some(std::fs::read(&path).unwrap()));
                }
            }
        }
        out
    }

    fn app() -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        app.manage(Arc::new(AppState::with_backups(
            tempfile::tempdir().unwrap().keep(),
        )));
        app
    }

    fn device_spec(path: &Path, format: &str, playlist: &str) -> ViewSpecDto {
        ViewSpecDto {
            source: TrackSourceDto::Device {
                path: path.to_string_lossy().into_owned(),
                format: format.into(),
                playlist: playlist.into(),
            },
            sort: "trackNo".into(),
            descending: false,
            query: String::new(),
            search_field: rbl_index::SearchField::All,
            filter: TrackFilterDto::default(),
        }
    }

    fn assert_unfinished<T: std::fmt::Debug>(what: &str, result: AppResult<T>) {
        match result {
            Err(error) => assert_eq!(error.kind, ErrorKind::UnfinishedWrite, "{what}: {error:?}"),
            Ok(found) => panic!("{what}: read a Stick with an unfinished write: {found:?}"),
        }
    }

    /// Every way the window reads a Stick, in either layout, on a Stick whose
    /// last write did not finish: each reports it, and not a byte changes.
    #[test]
    fn no_read_of_a_stick_settles_an_unfinished_write() {
        let (_src, dest) = stick();
        let mount = dest.path();
        interrupt(mount);
        let before = snapshot(mount);
        let path = mount.to_string_lossy().into_owned();
        let app = app();
        let state = app.state::<Arc<AppState>>();

        // The old tree: expanding a Stick, and every later listing refresh.
        assert_unfinished(
            "device_libraries",
            run(crate::device_library::device_libraries(path.clone())),
        );
        assert_unfinished(
            "device_libraries again",
            run(crate::device_library::device_libraries(path.clone())),
        );
        for format in ["deviceLibrary", "oneLibrary"] {
            // Opening a Stick playlist's tracks, and a library's All Tracks.
            for playlist in ["11", "0"] {
                assert_unfinished(
                    "open_view",
                    run(crate::commands::open_view(
                        state.clone(),
                        device_spec(mount, format, playlist),
                    )),
                );
            }
            assert_unfinished(
                "device_playlist_duplicates",
                run(crate::device_library::device_playlist_duplicates(
                    path.clone(),
                    format.into(),
                    "11".into(),
                    vec![],
                )),
            );
            assert_unfinished(
                "device_playlist_collection_tracks",
                run(crate::device_library::device_playlist_collection_tracks(
                    state.clone(),
                    path.clone(),
                    format.into(),
                    vec!["1".into()],
                )),
            );
            assert_unfinished(
                "device_playlist_import_plan",
                run(crate::device_library::device_playlist_import_plan(
                    state.clone(),
                    path.clone(),
                    format.into(),
                    "11".into(),
                    None,
                )),
            );
        }
        // The Stick's panel, in either layout.
        assert_unfinished(
            "device_settings",
            run(crate::device_settings::device_settings(path.clone())),
        );
        // The shell's tree.
        let outline = run(crate::stick_outline::stick_outline(path)).unwrap();
        assert!(outline.pending);

        assert_eq!(
            snapshot(mount),
            before,
            "not a byte on the Stick changed, the journal included"
        );
        assert!(rbl_export::recovery_pending(mount));
    }

    #[test]
    fn the_settle_action_settles_the_unfinished_write() {
        let (_src, dest) = stick();
        let mount = dest.path();
        interrupt(mount);
        let before = snapshot(mount);

        run(stick_settle(mount.to_string_lossy().into_owned())).unwrap();

        assert_ne!(snapshot(mount), before, "the Stick was settled");
        assert!(!rbl_export::recovery_pending(mount));
        let read = run(crate::device_library::device_libraries(
            mount.to_string_lossy().into_owned(),
        ));
        assert!(read.is_ok_or_not_unfinished(), "{read:?}");
    }

    #[test]
    fn settling_is_refused_while_rekordbox_runs_and_changes_nothing() {
        let (_src, dest) = stick();
        let mount = dest.path();
        interrupt(mount);
        let before = snapshot(mount);
        let refused = settle(mount, true).unwrap_err();
        assert_eq!(refused.kind, ErrorKind::ReadOnly);
        assert_eq!(snapshot(mount), before);
    }

    #[test]
    fn a_settled_stick_reads_as_before() {
        let (_src, dest) = stick();
        let mount = dest.path();
        let before = snapshot(mount);
        let path = mount.to_string_lossy().into_owned();
        let libraries = run(crate::device_library::device_libraries(path.clone())).unwrap();
        assert_eq!(libraries.len(), 2);
        assert_eq!(
            libraries[0]
                .nodes
                .iter()
                .map(|n| n.name.as_str())
                .collect::<Vec<_>>(),
            ["Sets", "Friday", "Warm Up"]
        );
        let friday = libraries[0]
            .nodes
            .iter()
            .find(|n| n.name == "Friday")
            .unwrap()
            .id
            .clone();
        let app = app();
        let handle = run(crate::commands::open_view(
            app.state::<Arc<AppState>>(),
            device_spec(mount, "deviceLibrary", &friday),
        ))
        .unwrap();
        assert_eq!(handle.len, 2);
        assert!(
            run(crate::device_settings::device_settings(path))
                .unwrap()
                .has_device_library
        );
        assert_eq!(
            snapshot(mount),
            before,
            "reading a settled Stick writes nothing"
        );
    }

    trait NotUnfinished {
        fn is_ok_or_not_unfinished(&self) -> bool;
    }
    impl<T> NotUnfinished for AppResult<T> {
        fn is_ok_or_not_unfinished(&self) -> bool {
            self.as_ref()
                .err()
                .is_none_or(|e| e.kind != ErrorKind::UnfinishedWrite)
        }
    }
}
