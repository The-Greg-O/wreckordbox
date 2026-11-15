//! The window's backup and a separation that finishes while it runs
//! (wbx-hy4): Stem audio is what the backup's own Sidecar snapshot names,
//! never what the `stems/` folder holds when it is read, so a restore is a
//! state that existed.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use wreckordbox_sidecar::stems::{NewStems, Role};
use wreckordbox_sidecar::{Fingerprint, Sidecar, Source, SourceKind};

use super::*;

fn fixture() -> (tempfile::TempDir, AppState, rbl_db::LibraryLocation) {
    let dir = tempfile::tempdir().unwrap();
    let location = rbl_db::fixture::build(dir.path(), rbl_db::fixture::Shape::default()).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (library, _) = rbl_index::load(&db).unwrap();
    let state = AppState::unprotected(dir.path().join("backups"));
    state.set_library(library, false, db.schema().db_version, 0, location.clone());
    (dir, state, location)
}

/// One second of silent 16-bit stereo WAV: what the fake backend writes.
fn write_wav(path: &Path) {
    let frames: u32 = 44_100;
    let data_len = frames * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&44_100_u32.to_le_bytes());
    out.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    out.extend_from_slice(&4_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(44 + data_len as usize, 0);
    fs::write(path, out).unwrap();
}

/// A whole separation of `track`, as the engine's worker records it (no
/// real separation is run: four files of the fake backend's kind): its Run
/// begun, its files published into the Run's folder, its Fact recorded,
/// its Run finished. The files it recorded.
fn a_separation(sidecar: &Sidecar, scratch: &Path, track: &str) -> Vec<String> {
    let source = Source::new(SourceKind::Tool, "separation-engine");
    let run = sidecar.begin_run(&source, "Separate (a stand-in)").unwrap();
    let work = scratch.join(format!("work-{}", run.0));
    fs::create_dir_all(&work).unwrap();
    let files: Vec<(Role, PathBuf)> = Role::ALL
        .into_iter()
        .map(|role| {
            let path = work.join(format!("{}.wav", role.name()));
            write_wav(&path);
            (role, path)
        })
        .collect();
    let set = sidecar
        .record_stems(&NewStems {
            track_id: track,
            fingerprint: &Fingerprint::from_stored(format!("a2:fake-{track}")),
            parent_samples: 44_100,
            sample_rate: 44_100,
            files: &files,
            source: &source,
            produced_by: serde_json::json!({"backend": "fake"}),
            run,
        })
        .unwrap();
    sidecar.finish_run(run, "done").unwrap();
    set.stems.into_iter().map(|stem| stem.path.strip_prefix(sidecar.dir()).unwrap().to_string_lossy().into_owned()).collect()
}

/// The Run a Stem file is in: `stems/<track>/run-<id>/<role>.wav`.
fn run_of(file: &str) -> i64 {
    let folder = file.split('/').find_map(|part| part.strip_prefix("run-")).unwrap_or_else(|| panic!("{file} is in no Run's folder"));
    folder.parse().unwrap()
}

fn runs_in(database: &Path) -> BTreeSet<i64> {
    let db = rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut statement = db.prepare("SELECT id FROM run").unwrap();
    let ids = statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
    ids
}

fn files_under(root: &Path, folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                found.push(entry.path().strip_prefix(root).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    found.sort();
    found
}

#[test]
fn a_separation_finishing_during_a_backup_leaves_a_restore_whose_stems_all_belong_to_its_runs() {
    let (dir, state, location) = fixture();
    let sidecar_dir = rbl_backup::sidecar_part::dir(&location);
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    let earlier = a_separation(&sidecar, dir.path(), "10001");
    // Stem audio is asked for in this window's backups.
    fs::create_dir_all(state.backup_dir()).unwrap();
    fs::write(state.backup_dir().join(rbl_backup::sidecar_part::OPTIONS_FILE), br#"{"stems": true}"#).unwrap();

    // The backup has snapshotted the Sidecar's store and not yet gathered
    // its Stems when a second separation finishes: its Run, files and
    // Fact all after the snapshot.
    let held = crate::test_holds::arm(crate::test_holds::BACKUP_SIDECAR_SNAPSHOTTED, state.backup_dir());
    let (archive, later) = std::thread::scope(|scope| {
        let backup = scope.spawn(|| create(&state));
        held.reached();
        let later = a_separation(&sidecar, dir.path(), "10002");
        held.release();
        (PathBuf::from(backup.join().unwrap().unwrap()), later)
    });

    // What the archive holds: every Stem file is of a Run its own Sidecar
    // has, and every one that Sidecar names is there.
    let zip = rbl_backup::archive::open(&archive).unwrap();
    let entries = rbl_backup::archive::names(&zip);
    let mut held_stems: Vec<&str> = entries.iter().filter_map(|name| name.strip_prefix("sidecar/")).filter(|name| name.starts_with("stems/") && !name.ends_with('/')).collect();
    held_stems.sort_unstable();
    let mut expected: Vec<&str> = earlier.iter().map(String::as_str).collect();
    expected.sort_unstable();
    assert_eq!(held_stems, expected, "only the Stems the backup's snapshot names: {entries:#?}");
    drop(zip);

    // Its restore: the Stems on disk are exactly those of Runs the restored
    // Sidecar has; the later separation's are gone with its Run.
    drop(sidecar);
    state.drop_reader();
    rbl_backup::restore::restore(
        &rbl_backup::restore::Request { archive: &archive, parts: rbl_backup::restore::Parts::ALL, location: &location, state_dir: state.backup_dir(), sizes: None },
        &|| Ok(()),
        &mut |_, _, _, _| Ok(()),
    )
    .unwrap();
    let runs = runs_in(&sidecar_dir.join("sidecar.db"));
    let on_disk = files_under(&sidecar_dir, &sidecar_dir.join("stems"));
    assert!(!on_disk.is_empty());
    for file in &on_disk {
        assert!(runs.contains(&run_of(file)), "{file} is of a Run the restored Sidecar does not have ({runs:?})");
    }
    assert_eq!(on_disk, expected);
    for file in &later {
        assert!(!sidecar_dir.join(file).exists(), "{file} came back without its Run");
    }
}
