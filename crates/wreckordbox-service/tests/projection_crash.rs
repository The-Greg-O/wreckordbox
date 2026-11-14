//! Projection (wbx-26): a hand set over a mixed selection, cut off by a
//! crash between recording its changes and recording its assertion. The
//! hosted service runs in a child process (this test, run again), which the
//! Ledger fault `abort_before_assert` kills; the parent then starts a
//! service on the same Library and Sidecar and projects.
//!
//! In a test binary of its own: it sets the Sidecar's folder and the
//! Ledger's faults in the process environment.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use rbl_db::{fixture, Library, LibraryLocation, OpenMode};
use rbl_library::history::TRACK_EDIT;
use rbl_library::{LibraryCore, NoHost, Touched};
use serde_json::{json, Value};
use wreckordbox_service::{ledger::FAULTS_ENV, stems::SIDECAR_DIR_ENV, Config, Service};

const CRASH_ROOT: &str = "WBX26_CRASH_ROOT";
const NAME: &str = "a_mixed_hand_set_cut_off_before_its_assertion_still_protects_the_unchanged_tracks";

fn location(root: &Path) -> LibraryLocation {
    LibraryLocation {
        master_db: root.join("library/master.db"),
        share_root: root.join("library/share"),
        passphrase: fixture::FIXTURE_PASSPHRASE.into(),
        is_real_install: false,
    }
}

fn hosted(root: &Path) -> (Service, Arc<LibraryCore>) {
    std::env::set_var(SIDECAR_DIR_ENV, root.join("sidecar"));
    let location = location(root);
    let config = Config {
        runtime_dir: root.join("run"),
        idle_exit: None,
        recovery_dir: root.join("recovery"),
    };
    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
    core.set_protection(false).unwrap();
    let db = Library::open(location.clone(), OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    core.set_library(&NoHost, index, false, db.schema().db_version, 0, location.clone());
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let service = Service::host(&location, &config, &core, &host).unwrap();
    (service, core)
}

fn record(service: &Service, track: &str, value: u8, confidence: f64) {
    service
        .call(
            "facts.record",
            json!({"source": {"kind": "agent", "name": "tagger"},
                   "facts": [{"track": track, "kind": "energy", "value": value,
                              "confidence": confidence}]}),
        )
        .unwrap();
}

fn stars(core: &LibraryCore, track: &str) -> u8 {
    let index = core.library().unwrap();
    index.rating[index.row_of(track).unwrap() as usize]
}

#[test]
fn a_mixed_hand_set_cut_off_before_its_assertion_still_protects_the_unchanged_tracks() {
    let [a, b, c] = [1, 2, 3].map(fixture::track_id);
    if let Ok(root) = std::env::var(CRASH_ROOT) {
        // The child: project 3 onto A and C, then the DJ, in the window,
        // sets 3 on A (already 3) and B (unrated). The service dies after
        // B's change is recorded and before A's assertion is.
        let root = Path::new(&root);
        let (service, _) = hosted(root);
        record(&service, &a, 3, 0.6);
        record(&service, &c, 3, 0.6);
        service.call("projection.apply", json!({})).unwrap();
        service.stop();
        drop(service);
        std::env::set_var(FAULTS_ENV, "abort_before_assert");
        let (_service, core) = hosted(root);
        let tracks = vec![a.clone(), b.clone()];
        core.asserting_edit(&NoHost, Touched::Metadata(tracks.clone()), TRACK_EDIT, |writer| {
            let mut edits = Vec::new();
            for track in &tracks {
                edits.push(writer.set_rating_with_undo(track, 3)?.1);
            }
            Ok(((), edits))
        })
        .unwrap();
        panic!("the fault did not abort");
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("library")).unwrap();
    std::fs::create_dir_all(dir.path().join("sidecar")).unwrap();
    std::fs::write(
        dir.path().join("sidecar/projection-policy.toml"),
        "[energy]\napply = 0.5\n",
    )
    .unwrap();
    fixture::build(
        &dir.path().join("library"),
        fixture::Shape {
            tracks: 5,
            ..fixture::Shape::default()
        },
    )
    .unwrap();
    let crashed = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture"])
        .env(CRASH_ROOT, dir.path())
        .output()
        .unwrap();
    assert_eq!(
        crashed.status.signal(),
        Some(6),
        "{}",
        String::from_utf8_lossy(&crashed.stderr)
    );

    // The next service: B's change is there; A's assertion never was.
    let (service, core) = hosted(dir.path());
    assert_eq!((stars(&core, &a), stars(&core, &b), stars(&core, &c)), (3, 3, 3));
    for track in [&a, &b, &c] {
        record(&service, track, 4, 0.9);
    }
    let applied: Value = service.call("projection.apply", json!({})).unwrap();
    // C, outside the selection, is still the projection's to replace.
    let changed: Vec<&str> = applied["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["track"].as_str().unwrap())
        .collect();
    assert_eq!(changed, [c.as_str()], "{applied}");
    // A and B are the DJ's: the DJ's chosen 3 stays.
    assert_eq!((stars(&core, &a), stars(&core, &b), stars(&core, &c)), (3, 3, 4));
}
