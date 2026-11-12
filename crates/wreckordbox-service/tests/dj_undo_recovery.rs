//! Crash between native Undo/Redo and its Ledger entry, in a hosted service.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use rbl_db::{fixture, Library, LibraryLocation, OpenMode};
use rbl_library::{
    history::{LibraryEdit, TRACK_EDIT},
    LibraryCore, NoHost, Touched,
};
use serde_json::json;
use std::{path::Path, process::Command, sync::Arc};
use wreckordbox_service::{ledger::FAULTS_ENV, stems::SIDECAR_DIR_ENV, Config, Service};

fn location(root: &Path) -> LibraryLocation {
    LibraryLocation {
        master_db: root.join("library/master.db"),
        share_root: root.join("library/share"),
        passphrase: fixture::FIXTURE_PASSPHRASE.into(),
        is_real_install: false,
    }
}
fn host(
    root: &Path,
    location: &LibraryLocation,
) -> (
    Config,
    Arc<LibraryCore>,
    Arc<dyn rbl_library::Host + Send + Sync>,
) {
    std::env::set_var(SIDECAR_DIR_ENV, root.join("sidecar"));
    let config = Config {
        runtime_dir: root.join("run"),
        idle_exit: None,
        recovery_dir: root.join("recovery"),
    };
    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
    core.set_protection(false).unwrap();
    let db = Library::open(location.clone(), OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    core.set_library(
        &NoHost,
        index,
        false,
        db.schema().db_version,
        0,
        location.clone(),
    );
    (config, core, Arc::new(NoHost))
}
fn rate(core: &LibraryCore, rating: u8) {
    core.recorded_edit(
        &NoHost,
        Touched::Metadata(vec![fixture::track_id(1)]),
        TRACK_EDIT,
        |w| {
            let (_, edit) = w.set_rating_with_undo(&fixture::track_id(1), rating)?;
            Ok(((), LibraryEdit::Track(vec![edit])))
        },
    )
    .unwrap();
}
#[test]
fn interrupted_undo_and_redo_reconcile_only_the_original_edits_fields() {
    if let Ok(root) = std::env::var("WBX19_CRASH_ROOT") {
        let root = Path::new(&root);
        let location = location(root);
        let (config, core, host) = host(root, &location);
        let service = Service::host(&location, &config, &core, &host).unwrap();
        rate(&core, 3);
        rate(&core, 2);
        if std::env::var("WBX19_CRASH_STEP").unwrap() == "redo" {
            core.step_history(&NoHost, true).unwrap();
        }
        service.stop();
        drop(service);
        std::env::set_var(FAULTS_ENV, "abort_after_write");
        let _service = Service::host(&location, &config, &core, &host).unwrap();
        core.step_history(
            &NoHost,
            std::env::var("WBX19_CRASH_STEP").unwrap() == "undo",
        )
        .unwrap();
        panic!("fault did not abort");
    }
    for step in ["undo", "redo"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("library")).unwrap();
        let location = fixture::build(
            &dir.path().join("library"),
            fixture::Shape {
                tracks: 4,
                ..fixture::Shape::default()
            },
        )
        .unwrap();
        let crashed = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "interrupted_undo_and_redo_reconcile_only_the_original_edits_fields",
                "--nocapture",
            ])
            .env("WBX19_CRASH_ROOT", dir.path())
            .env("WBX19_CRASH_STEP", step)
            .output()
            .unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            crashed.status.signal(),
            Some(6),
            "{}",
            String::from_utf8_lossy(&crashed.stderr)
        );
        let (config, core, host) = host(dir.path(), &location);
        let service = Service::host(&location, &config, &core, &host).unwrap();
        let fact = service
            .call(
                "facts.standing",
                json!({"id":fixture::track_id(1),"kind":"energy"}),
            )
            .unwrap()["standing"]
            .clone();
        assert_eq!(
            fact["value"],
            if step == "undo" { 3 } else { 2 },
            "{step}: {fact}"
        );
        assert_eq!(fact["source"]["kind"], "dj");
        if step == "redo" {
            assert_eq!(fact["reconciled"], true);
        } else {
            assert_eq!(fact["id"], 1);
        }
        assert!(service
            .call(
                "facts.standing",
                json!({"id":fixture::track_id(2),"kind":"energy"})
            )
            .unwrap()["standing"]
            .is_null());
        service.stop();
    }
}
