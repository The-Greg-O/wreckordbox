//! File-only edits through the shell, observed from the real CLI and a second client.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::state::AppState;
use rbl_db::fixture::{self, Shape};
use serde_json::{json, Value};
use std::{path::PathBuf, process::Command, sync::Arc, time::Duration};
use wreckordbox_service::{Client, Config, Service};

struct Fixture {
    service: Service,
    state: Arc<AppState>,
    location: rbl_db::LibraryLocation,
    config: Config,
    dat: PathBuf,
    ext: PathBuf,
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new(tracks: usize) -> Self {
        let dir = match std::env::var_os("WBX22_CHILD_ROOT") {
            Some(root) => tempfile::tempdir_in(root).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let root = dir.path().join("library");
        std::fs::create_dir_all(&root).unwrap();
        let location = fixture::build(
            &root,
            Shape {
                tracks,
                ..Shape::default()
            },
        )
        .unwrap();
        let relative = "/PIONEER/USBANLZ/file-ledger/ANLZ0000.DAT";
        fixture::set_analysis_path(&location, 1, relative).unwrap();
        fixture::write_options_json(
            &dir.path().join("options.json"),
            location.master_db.to_str().unwrap(),
            &location.passphrase,
        )
        .unwrap();
        let dat = rbl_anlz::resolve(&location.share_root, relative);
        let ext = rbl_anlz::sibling(&dat, "EXT");
        std::fs::create_dir_all(dat.parent().unwrap()).unwrap();
        let mut builder = rbl_anlz::AnlzBuilder::new();
        builder
            .path("/not-a-real-audio-file.wav")
            .beat_grid(&[rbl_anlz::Beat {
                beat_number: 1,
                tempo_x100: 12800,
                time_ms: 500,
            }]);
        std::fs::write(&dat, builder.finish()).unwrap();
        let mut file = rbl_anlz::Anlz::read(&dat).unwrap();
        let mut header = vec![0; 20];
        header[..4].copy_from_slice(&24_u32.to_be_bytes());
        header[4..6].copy_from_slice(&3_u16.to_be_bytes());
        header[6..8].copy_from_slice(&2_u16.to_be_bytes());
        header[14..16].copy_from_slice(&64_u16.to_be_bytes());
        let mut body = Vec::new();
        for (index, beat, kind) in [(1_u16, 1_u16, 1_u16), (2, 17, 2), (3, 33, 3)] {
            let mut entry = vec![0; 24];
            entry[..2].copy_from_slice(&index.to_be_bytes());
            entry[2..4].copy_from_slice(&beat.to_be_bytes());
            entry[4..6].copy_from_slice(&kind.to_be_bytes());
            body.extend(entry);
        }
        file.sections
            .push(rbl_anlz::Section::new(b"PSSI", header, body));
        std::fs::write(&ext, file.to_bytes()).unwrap();
        let config = Config {
            runtime_dir: dir.path().join("run"),
            idle_exit: None,
            recovery_dir: dir.path().join("recovery"),
        };
        let state = Arc::new(AppState::unprotected(config.recovery_dir.clone()));
        let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let (library, _) = rbl_index::load(&db).unwrap();
        state.set_library(library, false, db.schema().db_version, 0, location.clone());
        let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(rbl_library::NoHost);
        let service = Service::host(&location, &config, &state.shared_core(), &host).unwrap();
        Self {
            service,
            state,
            location,
            config,
            dat,
            ext,
            dir,
        }
    }

    fn cli(&self, args: &[&str]) -> Value {
        cli_at(self.dir.path(), args)
    }

    fn phrase(&self, edit: rbl_anlz::PhraseEdit) -> crate::AppResult<bool> {
        crate::analysis::phrase_edit(
            &self.state,
            &self.state.library().unwrap(),
            &self.location.share_root,
            &fixture::track_id(1),
            edit,
        )
    }

    fn grid(&self, offset: i16) -> crate::AppResult<()> {
        crate::link::save_grid_offset(&self.state, &fixture::track_id(1), offset)
    }
}

fn cli_at(dir: &std::path::Path, args: &[&str]) -> Value {
    let exe = std::env::current_exe().unwrap();
    let binary = exe.parent().unwrap().parent().unwrap().join("wreckordbox");
    assert!(
        binary.is_file(),
        "build the CLI first: cargo build -p wreckordbox"
    );
    let output = Command::new(binary)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", dir.join("home"))
        .env("RBXPORT_TEST", "1")
        .env("RBXPORT_OPTIONS", dir.join("options.json"))
        .env("RBXPORT_TEST_FIXTURE", dir.join("library"))
        .env("RBXPORT_STATE_DIR", dir.join("recovery"))
        .env("WRECKORDBOX_RUNTIME_DIR", dir.join("run"))
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn file_only_edits_are_named_once_in_the_window_run() {
    let f = Fixture::new(3);
    let mut watcher = Client::connect(&f.config.socket_path(&f.location)).unwrap();
    watcher.subscribe().unwrap();
    let before = std::fs::read(&f.ext).unwrap();
    assert!(f.phrase(rbl_anlz::PhraseEdit::Cut { beat: 25 }).unwrap());
    assert_ne!(std::fs::read(&f.ext).unwrap(), before);
    assert!(f.phrase(rbl_anlz::PhraseEdit::Clear { beat: 25 }).unwrap());
    f.grid(234).unwrap();
    assert_eq!(
        rbl_anlz::Anlz::read(&f.dat).unwrap().grid_offset(),
        Some(234)
    );
    let runs = f.cli(&["runs", "list"]);
    assert_eq!(runs["total"], 1, "file-only writes need a Run");
    let run = &runs["runs"][0];
    assert_eq!(run["description"], "Edits in the window");
    let entries = f.cli(&["runs", "show", &run["id"].to_string()]);
    assert_eq!(entries["total"], 3);
    for (entry, kind) in entries["entries"].as_array().unwrap().iter().zip([
        "phrase_cut",
        "phrase_clear",
        "link_grid_offset",
    ]) {
        assert_eq!(entry["kind"], kind);
        assert_eq!(entry["run"], run["id"]);
        assert_eq!(entry["source"], json!({"kind":"dj", "name":"dj"}));
        assert_eq!(entry["action"], "write");
        assert_eq!(entry["reversible"], false);
        assert!(entry["label"]
            .as_str()
            .unwrap()
            .contains(&fixture::track_id(1)));
        assert!(entry["label"]
            .as_str()
            .unwrap()
            .contains("previous analysis bytes were not retained"));
    }
    // File-only edits still do not refresh database rows or generate a burst
    // of Library events merely to record provenance.
    assert!(watcher
        .next_event(Duration::from_millis(100))
        .unwrap()
        .is_none());
    assert!(f.state.core().history().lock().undo_label().is_none());
    f.service.stop();
}

#[test]
fn refused_and_unchanged_file_edits_leave_no_run_or_intent() {
    let f = Fixture::new(3);
    let dat = std::fs::read(&f.dat).unwrap();
    let ext = std::fs::read(&f.ext).unwrap();
    f.state.core().set_protection(true).unwrap();
    assert_eq!(
        f.grid(234).unwrap_err().message,
        rbl_library::protection::MESSAGE
    );
    assert_eq!(
        f.phrase(rbl_anlz::PhraseEdit::Cut { beat: 25 })
            .unwrap_err()
            .message,
        rbl_library::protection::MESSAGE
    );
    assert_eq!(std::fs::read(&f.dat).unwrap(), dat);
    assert_eq!(std::fs::read(&f.ext).unwrap(), ext);
    f.state.core().set_protection(false).unwrap();
    assert!(!f.phrase(rbl_anlz::PhraseEdit::Cut { beat: 17 }).unwrap());
    assert!(crate::link::save_grid_offset(&f.state, "missing", 234).is_err());
    assert_eq!(f.cli(&["runs", "list"])["total"], 0);
    assert!(!f
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar")
        .exists());
}

#[test]
fn ledger_admission_failure_leaves_files_and_the_file_journal_alone() {
    let f = Fixture::new(3);
    let dat = std::fs::read(&f.dat).unwrap();
    let ext = std::fs::read(&f.ext).unwrap();
    let stamp = std::fs::metadata(&f.dat).unwrap().modified().unwrap();
    // A file where the Sidecar's directory should be makes admission fail.
    let sidecar = f
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar");
    std::fs::write(sidecar, b"not a directory").unwrap();
    assert!(f
        .grid(234)
        .unwrap_err()
        .message
        .contains("Ledger cannot record"));
    assert!(f
        .phrase(rbl_anlz::PhraseEdit::Cut { beat: 25 })
        .unwrap_err()
        .message
        .contains("Ledger cannot record"));
    assert_eq!(std::fs::read(&f.dat).unwrap(), dat);
    assert_eq!(std::fs::read(&f.ext).unwrap(), ext);
    assert_eq!(
        std::fs::metadata(&f.dat).unwrap().modified().unwrap(),
        stamp
    );
    assert!(!f.config.recovery_dir.join("analysis-journal").exists());
}

#[test]
fn file_guard_on_a_protected_stick_refuses_before_publication() {
    let f = Fixture::new(3);
    let before = std::fs::read(&f.ext).unwrap();
    // A temporary fixture stands for a Stick; no real Stick is opened or
    // unlocked. Test the common guard independently of Ledger availability.
    std::fs::write(
        f.location
            .master_db
            .parent()
            .unwrap()
            .join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let result = f.state.core().file_edit().map(|_edit| {
        std::fs::write(&f.ext, b"a file-only change").unwrap();
    });
    assert!(
        result.as_ref().is_err_and(rbl_library::stick::refused),
        "{result:?}"
    );
    assert_eq!(std::fs::read(&f.ext).unwrap(), before);
    assert!(!f
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar")
        .exists());
}

// Each fault runs in a separate process: environment changes never affect
// neighbouring tests, and a real process death tests the durable intent.
#[test]
#[ignore = "subprocess helper for the crash and spill regression"]
fn file_ledger_fault_child() {
    let mode = std::env::var("WBX22_CHILD_MODE").unwrap();
    let f = Fixture::new(3);
    std::fs::write(
        std::path::PathBuf::from(std::env::var_os("WBX22_CHILD_ROOT").unwrap())
            .join("fixture.json"),
        serde_json::to_vec(&json!({"dir":f.dir.path(), "dat":f.dat})).unwrap(),
    )
    .unwrap();
    f.grid(234).unwrap();
    assert_eq!(
        rbl_anlz::Anlz::read(&f.dat).unwrap().grid_offset(),
        Some(234)
    );
    assert_ne!(
        mode, "crash",
        "the crash fault must have terminated the process"
    );
    let spill = f
        .location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar/ledger-spill.jsonl");
    assert!(spill.is_file(), "the failed append is kept durably");
    // The next edit replays the kept entry without duplicating it and still
    // succeeds. Both edits belong to the window's one Run.
    assert!(f.phrase(rbl_anlz::PhraseEdit::Cut { beat: 25 }).unwrap());
    assert!(!spill.exists());
    let runs = f.cli(&["runs", "list"]);
    assert_eq!(runs["total"], 1);
    let entries = f.cli(&["runs", "show", &runs["runs"][0]["id"].to_string()]);
    assert_eq!(entries["total"], 2);
    assert_eq!(entries["entries"][0]["kind"], "link_grid_offset");
    assert_eq!(entries["entries"][1]["kind"], "phrase_cut");
    assert_ne!(entries["entries"][0]["id"], entries["entries"][1]["id"]);
}

#[test]
fn file_edits_survive_an_append_failure_and_a_crash_leaves_an_identified_intent() {
    for (mode, fault) in [("spill", "fail_append=1"), ("crash", "abort_after_write")] {
        let root = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "file_ledger_tests::file_ledger_fault_child",
                "--ignored",
                "--nocapture",
            ])
            .env("WBX22_CHILD_ROOT", root.path())
            .env("WBX22_CHILD_MODE", mode)
            .env(wreckordbox_service::ledger::FAULTS_ENV, fault)
            .output()
            .unwrap();
        if mode == "spill" {
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            continue;
        }
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            output.status.signal(),
            Some(6),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let fixture: Value =
            serde_json::from_slice(&std::fs::read(root.path().join("fixture.json")).unwrap())
                .unwrap();
        let dir = PathBuf::from(fixture["dir"].as_str().unwrap());
        let dat = PathBuf::from(fixture["dat"].as_str().unwrap());
        assert_eq!(rbl_anlz::Anlz::read(&dat).unwrap().grid_offset(), Some(234));
        let runs = cli_at(&dir, &["runs", "list"]);
        assert_eq!(runs["total"], 1);
        let entries = cli_at(&dir, &["runs", "show", &runs["runs"][0]["id"].to_string()]);
        assert_eq!(entries["total"], 1);
        let entry = &entries["entries"][0];
        assert_eq!(entry["kind"], "unconfirmed");
        assert_eq!(entry["reversible"], false);
        assert_eq!(
            entry["command"],
            json!({"name":"files.edit", "args":{"kind":"link_grid_offset", "track":fixture::track_id(1)}})
        );
        assert_eq!(entry["source"], json!({"kind":"dj", "name":"dj"}));
        cli_at(&dir, &["stop"]);
    }
}

#[test]
#[ignore = "explicit 20,000-track file-edit latency measurement"]
fn file_edit_latency_at_twenty_thousand_tracks() {
    let f = Fixture::new(20_000);
    let original = std::fs::read(&f.ext).unwrap();
    for kind in ["link_grid_offset", "phrase_cut"] {
        let mut times = Vec::new();
        for n in 1..=7 {
            // Fixture reset is outside the measured write and not a production path.
            if kind == "phrase_cut" {
                std::fs::write(&f.ext, &original).unwrap();
            }
            let begin = std::time::Instant::now();
            if kind == "phrase_cut" {
                assert!(f.phrase(rbl_anlz::PhraseEdit::Cut { beat: 25 }).unwrap());
            } else {
                f.grid(n).unwrap();
            }
            times.push(begin.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "WBX22_TIMING {kind} median_ms={:.3} samples={times:?}",
            times[3]
        );
    }
}
