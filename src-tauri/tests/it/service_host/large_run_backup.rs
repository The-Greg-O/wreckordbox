//! The backup before a large Run (wbx-25), with the window hosting the
//! service: the window's own edits and backups at the moments the race
//! needs (held there by the backup's test holds, `large_run::HOLDS`, in the
//! state folder). Temporary folders stand in for the Library and a Stick.

use super::*;

use std::path::Path;
use wreckordbox_service::large_run::HOLDS;

/// Arms the backup's hold `point` in the window's state folder.
fn arm(window: &Window, point: &str) {
    let dir = window.config().recovery_dir.join(HOLDS);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(point), b"").unwrap();
}

fn reached(window: &Window, point: &str) {
    let marker = window
        .config()
        .recovery_dir
        .join(HOLDS)
        .join(format!("{point}.reached"));
    let deadline = Instant::now() + Duration::from_secs(60);
    while !marker.exists() {
        assert!(
            Instant::now() < deadline,
            "the backup never reached {point}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn release(window: &Window, point: &str) {
    std::fs::remove_file(window.config().recovery_dir.join(HOLDS).join(point)).unwrap();
}

/// A command run to its end, or killed after `secs` (then `None`).
fn finished_within(mut child: std::process::Child, secs: u64) -> Option<Run> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if child.try_wait().unwrap().is_some() {
            return Some(finished(child.wait_with_output().unwrap()));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sidecar_dir(window: &Window) -> PathBuf {
    window
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar")
}

/// Three tracks with an agent's Energy waiting, above a threshold of two.
fn three_to_project(window: &Window) {
    std::fs::create_dir_all(sidecar_dir(window)).unwrap();
    std::fs::write(
        sidecar_dir(window).join("projection-policy.toml"),
        "[energy]\napply = 0.5\n\n[backup]\nabove_tracks = 2\n",
    )
    .unwrap();
    for (track, energy) in [(track_id(1), "4"), (track_id(2), "2"), (track_id(3), "5")] {
        window
            .cli(&[
                "--json",
                "facts",
                "record",
                &track,
                "energy",
                energy,
                "--source",
                "agent:tagger",
                "--confidence",
                "0.9",
            ])
            .succeeded();
    }
}

/// What a restored Library and Sidecar say about `track`'s rating: the
/// Library's value, how many Ledger entries changed its Rating, and its DJ
/// Facts of Energy.
fn restored_view(window: &Window, track: &str) -> (i64, i64, i64) {
    let db = rbl_db::Library::open(window.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let rating: i64 = db
        .connection()
        .query_row(
            "SELECT Rating FROM djmdContent WHERE ID = ?1",
            [track],
            |r| r.get(0),
        )
        .unwrap();
    let sidecar = rusqlite::Connection::open_with_flags(
        sidecar_dir(window).join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let entries: i64 = sidecar
        .query_row(
            "SELECT COUNT(*) FROM ledger, json_each(ledger.payload, '$.edits') AS edit
             WHERE ledger.kind = 'track'
               AND json_extract(edit.value, '$.track') = ?1
               AND json_extract(edit.value, '$.column') = 'Rating'",
            [track],
            |r| r.get(0),
        )
        .unwrap();
    let facts: i64 = sidecar
        .query_row(
            "SELECT COUNT(*) FROM fact WHERE track_id = ?1 AND kind = 'energy' AND source_kind = 'dj'",
            [track],
            |r| r.get(0),
        )
        .unwrap();
    (rating, entries, facts)
}

/// Restores `archive` with the real command, the window gone.
fn restore(window: &Window, archive: &str) {
    window.state().stop_service();
    // A command since may have started a service of its own.
    let _ = window.cli(&["stop"]);
    // The window's state folder, where its Library Protection is recorded.
    let restored = finished(
        window
            .command(None, &["--json", "backup", "restore", archive, "--yes"])
            .env("RBXPORT_STATE_DIR", window.config().recovery_dir)
            .output()
            .unwrap(),
    );
    assert!(restored.ok, "{}{}", restored.stdout, restored.stderr);
}

fn backups(folder: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("rbexport-")
        })
        .collect();
    found.sort();
    found
}

/// Review blocker 1: the DJ edits another track in the window while the
/// backup has read one store and not the other. The backup's Library and
/// Sidecar must be of one moment: restored, the edit is in both or neither.
#[test]
fn a_window_edit_between_the_two_stores_is_in_both_halves_of_the_backup_or_neither() {
    let window = window();
    window.host().unwrap();
    three_to_project(&window);
    let other = track_id(4);
    let before = restored_view(&window, &other);

    arm(&window, "between-stores");
    let apply = window
        .command(None, &["--json", "project"])
        .spawn()
        .unwrap();
    reached(&window, "between-stores");
    // The DJ sets the other track's rating now: at once when nothing holds
    // the edit gate, else as soon as it is free.
    let state = Arc::clone(window.state().inner());
    let handle = window.app.handle().clone();
    let free = state.core().edit_gate().try_lock().is_some();
    let edit = std::thread::spawn(move || {
        run(commands::set_track_rating(
            handle.clone(),
            handle.state(),
            vec![track_id(4)],
            5,
        ))
        .unwrap();
    });
    if free {
        edit.join().unwrap();
        release(&window, "between-stores");
    } else {
        release(&window, "between-stores");
        edit.join().unwrap();
    }
    let applied = finished_within(apply, 60)
        .expect("the apply finished")
        .json();
    let archive = applied["large_run"]["backup"]["path"]
        .as_str()
        .expect("a backup")
        .to_owned();

    restore(&window, &archive);
    let (rating, entries, facts) = restored_view(&window, &other);
    if rating == before.0 {
        assert_eq!(
            (entries, facts),
            (before.1, before.2),
            "the Library lacks the edit, so must the Sidecar"
        );
    } else {
        assert_eq!(rating, 5);
        assert_eq!(
            entries,
            before.1 + 1,
            "the Library has the edit, so must the Ledger"
        );
        assert_eq!(facts, before.2 + 1, "and the DJ's Fact of it");
    }
    // Neither half holds anything of the projection.
    assert_eq!(restored_view(&window, &track_id(1)).1, 0);
}

/// Review blocker 1, the binding: the DJ edits another track after the
/// backup and before the Run's first write. The backup the Run names is
/// of the Library right before it (taken again), not of an earlier moment.
#[test]
fn a_window_edit_after_the_backup_and_before_the_write_is_in_the_backup_the_run_names() {
    let window = window();
    window.host().unwrap();
    three_to_project(&window);
    let other = track_id(4);

    arm(&window, "backed-up");
    let apply = window
        .command(None, &["--json", "project"])
        .spawn()
        .unwrap();
    reached(&window, "backed-up");
    run(commands::set_track_rating(
        window.app.handle().clone(),
        window.state(),
        vec![other.clone()],
        5,
    ))
    .unwrap();
    release(&window, "backed-up");
    let applied = finished_within(apply, 60)
        .expect("the apply finished")
        .json();
    let archive = applied["large_run"]["backup"]["path"]
        .as_str()
        .expect("a backup")
        .to_owned();
    // The backup taken first was no longer of the moment before the Run:
    // only the one taken again is kept.
    assert_eq!(
        backups(&window.config().recovery_dir),
        vec![PathBuf::from(&archive)]
    );

    restore(&window, &archive);
    let (rating, entries, facts) = restored_view(&window, &other);
    assert_eq!(
        (rating, entries, facts),
        (5, 1, 1),
        "the edit made before the Run is in the backup"
    );
    assert_eq!(
        restored_view(&window, &track_id(1)).1,
        0,
        "and nothing of the Run"
    );
}

/// Review blocker 2: the window's backup and the service's, both to the
/// same Stick, one holding the Stick while the other holds the edit gate.
/// Both finish: every backup writer takes the edit gate, then the Stick,
/// then the analysis lock.
#[test]
fn the_windows_backup_and_a_large_runs_backup_to_one_stick_both_finish() {
    let window = window();
    let sticks = tempfile::tempdir().unwrap();
    let stick = sticks.path().join("GIGSTICK");
    std::fs::create_dir_all(stick.join("backups")).unwrap();
    std::fs::write(
        stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let state_dir = window.config().recovery_dir;
    wreckordbox_stick_protection::unlock(&state_dir, &stick, Some("GIGSTICK")).unwrap();
    run(commands::set_backup_directory(
        window.state(),
        stick.join("backups").to_string_lossy().into_owned(),
    ))
    .unwrap();
    window.host().unwrap();
    three_to_project(&window);

    arm(&window, "stick-held");
    let apply = window
        .command(None, &["--json", "project"])
        .spawn()
        .unwrap();
    reached(&window, "stick-held");
    // The DJ starts a backup in the window now.
    let state = Arc::clone(window.state().inner());
    let handle = window.app.handle().clone();
    let (done, finished_backup) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(run(commands::back_up_library(handle.state())));
    });
    // Until it holds the edit gate, and whatever it takes next.
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.core().edit_gate().try_lock().is_some() {
        assert!(Instant::now() < deadline, "the window's backup never began");
        std::thread::sleep(Duration::from_millis(2));
    }
    let settle = Instant::now() + Duration::from_millis(500);
    while state.core().analysis_write().try_lock().is_some() && Instant::now() < settle {
        std::thread::sleep(Duration::from_millis(2));
    }
    release(&window, "stick-held");

    let applied = finished_within(apply, 60);
    let window_backup = finished_backup.recv_timeout(Duration::from_secs(60));
    let applied = applied
        .expect("the large Run's backup and apply finished")
        .json();
    assert!(
        applied["large_run"]["backup"]["path"].is_string(),
        "{applied}"
    );
    let made = window_backup
        .expect("the window's backup finished")
        .unwrap();
    assert!(Path::new(&made).starts_with(&stick), "{made}");
    assert_eq!(backups(&stick.join("backups")).len(), 2);
}
