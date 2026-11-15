//! The window's backup and Stem audio (wbx-hy4): what a backup carries is
//! what its own Sidecar snapshot names, never what the `stems/` folder
//! holds when it is read, so a separation finishing mid-backup is left out
//! whole and a restore is a state that existed.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

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

/// One second of 16-bit stereo WAV, every sample byte `fill`: the shape
/// the fake backend writes, with contents a test can tell apart.
fn write_wav(path: &Path, fill: u8) {
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
    out.resize(44 + data_len as usize, fill);
    fs::write(path, out).unwrap();
}

fn fingerprint(track: &str) -> Fingerprint {
    Fingerprint::from_stored(format!("a2:fake-{track}"))
}

/// What a separation recorded: its Run and its files (paths in the
/// Sidecar) with their bytes.
struct Separation {
    run: i64,
    files: Vec<(String, Vec<u8>)>,
}

impl Separation {
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.files.iter().map(|(name, _)| name.clone()).collect();
        names.sort();
        names
    }
}

/// A whole separation of `track`, as the engine's worker records it (no
/// real separation is run: four files of the fake backend's kind, each its
/// own bytes): its Run begun, its files published into the Run's folder,
/// its Fact recorded, its Run finished.
fn a_separation(sidecar: &Sidecar, scratch: &Path, track: &str, fill: u8) -> Separation {
    let source = Source::new(SourceKind::Tool, "separation-engine");
    let run = sidecar.begin_run(&source, "Separate (a stand-in)").unwrap();
    let work = scratch.join(format!("work-{}", run.0));
    fs::create_dir_all(&work).unwrap();
    let files: Vec<(Role, PathBuf)> = Role::ALL
        .into_iter()
        .zip(fill..)
        .map(|(role, fill)| {
            let path = work.join(format!("{}.wav", role.name()));
            write_wav(&path, fill);
            (role, path)
        })
        .collect();
    let set = sidecar
        .record_stems(&NewStems {
            track_id: track,
            fingerprint: &fingerprint(track),
            parent_samples: 44_100,
            sample_rate: 44_100,
            files: &files,
            source: &source,
            produced_by: serde_json::json!({"backend": "fake"}),
            run,
        })
        .unwrap();
    sidecar.finish_run(run, "done").unwrap();
    let files = set
        .stems
        .into_iter()
        .map(|stem| {
            let name = stem
                .path
                .strip_prefix(sidecar.dir())
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (name, fs::read(&stem.path).unwrap())
        })
        .collect();
    Separation { run: run.0, files }
}

fn ask_for_stems(state: &AppState) {
    fs::create_dir_all(state.backup_dir()).unwrap();
    fs::write(
        state
            .backup_dir()
            .join(rbl_backup::sidecar_part::OPTIONS_FILE),
        br#"{"stems": true}"#,
    )
    .unwrap();
}

/// Every Stem file a store's Stems Facts name, with the Run of its Fact.
fn stems_facts(database: &Path) -> Vec<(i64, String)> {
    let db =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let mut statement = db
        .prepare(
            "SELECT fact.run_id, json_extract(stem.value, '$.file') FROM fact, json_each(fact.value, '$.stems') AS stem
             WHERE fact.kind = 'stems' ORDER BY 2",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

fn runs_in(database: &Path) -> BTreeSet<i64> {
    let db =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let mut statement = db.prepare("SELECT id FROM run").unwrap();
    let ids = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    ids
}

/// The Stem files an archive holds, as paths in the Sidecar, sorted.
fn archived_stems(archive: &Path) -> Vec<String> {
    let zip = rbl_backup::archive::open(archive).unwrap();
    let mut held: Vec<String> = rbl_backup::archive::names(&zip)
        .into_iter()
        .filter_map(|name| name.strip_prefix("sidecar/").map(str::to_owned))
        .filter(|name| name.starts_with("stems/") && !name.ends_with('/'))
        .collect();
    held.sort();
    held
}

/// One entry of an archive, read whole (a stored or deflated file).
fn archived(archive: &Path, name: &str) -> Vec<u8> {
    let mut zip = rbl_backup::archive::open(archive).unwrap();
    let mut entry = zip.by_name(name).unwrap();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
    bytes
}

/// The archive's own copy of the Sidecar's store, written out to `into`.
fn archived_store(archive: &Path, into: &Path) -> PathBuf {
    let path = into.join("archived-sidecar.db");
    fs::write(&path, archived(archive, "sidecar/sidecar.db")).unwrap();
    path
}

fn restore_all(state: &AppState, location: &rbl_db::LibraryLocation, archive: &Path) {
    state.drop_reader();
    rbl_backup::restore::restore(
        &rbl_backup::restore::Request {
            archive,
            parts: rbl_backup::restore::Parts::ALL,
            location,
            state_dir: state.backup_dir(),
            sizes: None,
        },
        &|| Ok(()),
        &mut |_, _, _, _| Ok(()),
    )
    .unwrap();
}

fn files_under(root: &Path, folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                found.push(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    found.sort();
    found
}

/// A backup, or what is left of one, in the backup folder: archives and
/// partial copies (set-aside `.discard-` names are deleted in the background).
fn backups_left(state: &AppState) -> Vec<String> {
    let mut left: Vec<String> = fs::read_dir(state.backup_destination())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.starts_with(".partial-")
                || (name.starts_with("rbexport-") && is_zip(Path::new(name)))
        })
        .collect();
    left.sort();
    left
}

/// A window backup held at `point` (after the Sidecar's store is
/// snapshotted) while `meanwhile` runs; its answer.
fn backup_held_at(state: &AppState, point: &str, meanwhile: impl FnOnce()) -> AppResult<String> {
    let held = crate::test_holds::arm(point, state.backup_dir());
    std::thread::scope(|scope| {
        let backup = scope.spawn(|| create(state));
        held.reached();
        meanwhile();
        held.release();
        backup.join().unwrap()
    })
}

#[test]
fn a_separation_finishing_during_a_backup_leaves_a_restore_whose_stems_all_belong_to_its_runs() {
    let (dir, state, location) = fixture();
    let sidecar_dir = rbl_backup::sidecar_part::dir(&location);
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    let earlier = a_separation(&sidecar, dir.path(), "10001", 1);
    ask_for_stems(&state);

    // The backup has snapshotted the Sidecar's store and not yet gathered
    // its Stems when a second separation finishes: its Run, files and
    // Fact all after the snapshot.
    let mut later = None;
    let archive = PathBuf::from(
        backup_held_at(
            &state,
            crate::test_holds::BACKUP_SIDECAR_SNAPSHOTTED,
            || {
                later = Some(a_separation(&sidecar, dir.path(), "10002", 11));
            },
        )
        .unwrap(),
    );
    let later = later.unwrap();

    // The archive: its store's Stems Facts name exactly the Stem files it
    // holds, all of the earlier Run, byte for byte; of the later Run
    // neither the Run, nor its Fact, nor a file.
    let store = archived_store(&archive, dir.path());
    let named = stems_facts(&store);
    let named_files: Vec<String> = named.iter().map(|(_, file)| file.clone()).collect();
    assert_eq!(
        archived_stems(&archive),
        named_files,
        "the archive's Stems are those its own store names"
    );
    assert_eq!(named_files, earlier.names());
    assert!(
        named.iter().all(|(run, _)| *run == earlier.run),
        "{named:?}"
    );
    for (file, bytes) in &earlier.files {
        assert_eq!(
            &archived(&archive, &format!("sidecar/{file}")),
            bytes,
            "{file}"
        );
    }
    let archived_runs = runs_in(&store);
    assert!(archived_runs.contains(&earlier.run));
    assert!(
        !archived_runs.contains(&later.run),
        "the later Run is in the archive: {archived_runs:?}"
    );

    // Its restore: the Stems on disk are exactly those the restored
    // Sidecar's Facts name, of Runs it has, with the earlier audio; the
    // later separation is gone whole, Run, Fact and files.
    drop(sidecar);
    restore_all(&state, &location, &archive);
    let restored = sidecar_dir.join("sidecar.db");
    let runs = runs_in(&restored);
    let facts = stems_facts(&restored);
    assert_eq!(facts, named);
    assert!(
        facts.iter().all(|(run, _)| runs.contains(run)),
        "{facts:?} {runs:?}"
    );
    assert!(!runs.contains(&later.run), "{runs:?}");
    assert_eq!(
        files_under(&sidecar_dir, &sidecar_dir.join("stems")),
        earlier.names()
    );
    for (file, bytes) in &earlier.files {
        assert_eq!(&fs::read(sidecar_dir.join(file)).unwrap(), bytes, "{file}");
    }
    for (file, _) in &later.files {
        assert!(
            !sidecar_dir.join(file).exists(),
            "{file} came back without its Run"
        );
    }
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    assert!(sidecar
        .current_stems("10001", &fingerprint("10001"))
        .unwrap()
        .is_some());
    assert_eq!(
        sidecar.stems("10002").unwrap().len(),
        0,
        "the later Stems' Fact came back"
    );
}

/// Review round 1, finding 1: a folder on the way to a recorded Stem file
/// is a symbolic link to a folder outside the Sidecar. The backup is
/// refused before it reads the file, and leaves nothing in its folder.
#[test]
fn a_stem_folder_that_is_a_symbolic_link_refuses_the_backup() {
    let (dir, state, location) = fixture();
    let sidecar_dir = rbl_backup::sidecar_part::dir(&location);
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    let recorded = a_separation(&sidecar, dir.path(), "10001", 1);
    ask_for_stems(&state);
    let run_folder = sidecar_dir.join(Path::new(&recorded.files[0].0).parent().unwrap());
    let outside = dir.path().join("outside-the-sidecar");
    fs::rename(&run_folder, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, &run_folder).unwrap();

    let refused = create(&state).unwrap_err();
    assert!(
        failure_message(&refused).contains("Symbolic links"),
        "{}",
        failure_message(&refused)
    );
    assert_eq!(backups_left(&state), Vec::<String>::new());
}

/// Review round 1, finding 2: stopping a backup while it gathers the
/// Stems its snapshot names stops it there, the next time it asks, and
/// leaves no partial copy in its folder.
#[test]
fn a_backup_stopped_while_it_gathers_stems_stops_there_and_leaves_no_partial_copy() {
    let (dir, state, location) = fixture();
    let sidecar_dir = rbl_backup::sidecar_part::dir(&location);
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    a_separation(&sidecar, dir.path(), "10001", 1);
    ask_for_stems(&state);

    let stop = AtomicBool::new(false);
    let stopped_at = parking_lot::Mutex::new(None::<String>);
    let held = crate::test_holds::arm(
        crate::test_holds::BACKUP_STEMS_GATHERING,
        state.backup_dir(),
    );
    let result = std::thread::scope(|scope| {
        let backup = scope.spawn(|| {
            create_with_progress(&state, &mut |_, _, _, item| {
                if stop.load(Ordering::Acquire) {
                    stopped_at.lock().get_or_insert_with(|| item.to_owned());
                    return Err(error("Backup stopped"));
                }
                Ok(())
            })
        });
        held.reached();
        stop.store(true, Ordering::Release);
        held.release();
        backup.join().unwrap()
    });
    assert!(result.is_err());
    assert_eq!(
        stopped_at.lock().as_deref(),
        Some("Scanning the Sidecar"),
        "the stop was seen only after the Stems were gathered"
    );
    assert_eq!(backups_left(&state), Vec::<String>::new());
}

/// Review round 1, finding 3 (a limitation, pinned): a recorded Stem file
/// removed after the snapshot is left out of the archive while its Fact
/// stays; the restore succeeds, and the track reads as needing separating.
#[test]
fn a_stem_file_removed_after_the_snapshot_is_left_out_and_restores_as_needing_separation() {
    let (dir, state, location) = fixture();
    let sidecar_dir = rbl_backup::sidecar_part::dir(&location);
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    let recorded = a_separation(&sidecar, dir.path(), "10001", 1);
    assert!(sidecar
        .current_stems("10001", &fingerprint("10001"))
        .unwrap()
        .is_some());
    ask_for_stems(&state);
    let (gone, _) = &recorded.files[0];

    let archive = PathBuf::from(
        backup_held_at(
            &state,
            crate::test_holds::BACKUP_SIDECAR_SNAPSHOTTED,
            || {
                fs::remove_file(sidecar_dir.join(gone)).unwrap();
            },
        )
        .unwrap(),
    );
    let held = archived_stems(&archive);
    assert!(!held.contains(gone), "{held:?}");
    assert_eq!(held.len(), 3, "{held:?}");
    let store = archived_store(&archive, dir.path());
    assert!(
        stems_facts(&store).iter().any(|(_, file)| file == gone),
        "the Fact naming it stays"
    );

    drop(sidecar);
    restore_all(&state, &location, &archive);
    assert!(!sidecar_dir.join(gone).exists());
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    assert!(
        sidecar
            .current_stems("10001", &fingerprint("10001"))
            .unwrap()
            .is_none(),
        "reads as needing separating"
    );
}

/// Review round 1, finding 3 (a limitation, pinned): a recorded Stem file
/// replaced after the snapshot is archived as it is then, not checked
/// against its Fact's hash; after a restore the track reads as needing
/// separating.
#[test]
fn a_stem_file_replaced_after_the_snapshot_is_archived_as_it_is_and_restores_as_needing_separation()
{
    let (dir, state, location) = fixture();
    let sidecar_dir = rbl_backup::sidecar_part::dir(&location);
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    let recorded = a_separation(&sidecar, dir.path(), "10001", 1);
    ask_for_stems(&state);
    let (changed, _) = &recorded.files[0];

    let archive = PathBuf::from(
        backup_held_at(
            &state,
            crate::test_holds::BACKUP_SIDECAR_SNAPSHOTTED,
            || {
                let replacement = dir.path().join("replacement.wav");
                write_wav(&replacement, 99);
                fs::rename(&replacement, sidecar_dir.join(changed)).unwrap();
            },
        )
        .unwrap(),
    );
    let replacement = fs::read(sidecar_dir.join(changed)).unwrap();
    assert_eq!(
        archived(&archive, &format!("sidecar/{changed}")),
        replacement
    );

    drop(sidecar);
    restore_all(&state, &location, &archive);
    assert_eq!(fs::read(sidecar_dir.join(changed)).unwrap(), replacement);
    let sidecar = Sidecar::open(&sidecar_dir).unwrap();
    assert!(
        sidecar
            .current_stems("10001", &fingerprint("10001"))
            .unwrap()
            .is_none(),
        "reads as needing separating"
    );
}
