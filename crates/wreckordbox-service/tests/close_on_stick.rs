//! A hosted service closing with its Sidecar on an unlocked Stick (wbx-8yh):
//! ending the window's Run is the Ledger's own housekeeping, kept on the
//! Stick while it is unlocked. Separate target: it sets the Sidecar's
//! folder in the process environment.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::sync::Arc;

use rbl_db::fixture::{self, Shape};
use rbl_library::history::{LibraryEdit, TRACK_EDIT};
use rbl_library::{LibraryCore, NoHost, Touched};
use wreckordbox_service::ledger::WINDOW_RUN;
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Config, Service};

#[test]
fn closing_on_an_unlocked_stick_ends_the_windows_run() {
    assert_eq!(std::env::var("RBXPORT_TEST").as_deref(), Ok("1"));
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 2,
            ..Shape::default()
        },
    )
    .unwrap();
    let sidecar = dir.path().join("sidecar");
    std::fs::create_dir_all(&sidecar).unwrap();
    std::env::set_var(SIDECAR_DIR_ENV, &sidecar);
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    // The Sidecar's folder is a Stick, unlocked by name.
    std::fs::write(
        sidecar.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    wreckordbox_stick_protection::unlock(&config.recovery_dir, &sidecar, Some("sidecar")).unwrap();

    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
    core.set_protection(false).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    core.set_library(
        &NoHost,
        index,
        false,
        db.schema().db_version,
        0,
        location.clone(),
    );
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let service = Service::host(&location, &config, &core, &host).unwrap();
    let track = fixture::track_id(1);
    core.recorded_edit(
        &NoHost,
        Touched::Metadata(vec![track.clone()]),
        TRACK_EDIT,
        |writer| {
            let (_, recorded) = writer.set_rating_with_undo(&track, 3)?;
            Ok(((), LibraryEdit::Track(vec![recorded])))
        },
    )
    .unwrap();
    service.stop();
    drop(service);

    let db = rusqlite::Connection::open_with_flags(
        sidecar.join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let outcome: Option<String> = db
        .query_row(
            "SELECT outcome FROM run WHERE description = ?1",
            [WINDOW_RUN],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        outcome.as_deref(),
        Some("wreckordbox stopped"),
        "the window's Run was ended as the service closed"
    );
}
