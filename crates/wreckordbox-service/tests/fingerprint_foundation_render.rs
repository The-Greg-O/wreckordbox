//! Separate process: service hosting uses the process-wide Sidecar location.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
#[path = "../../wreckordbox-sidecar/tests/fingerprint_support/mod.rs"]
mod fixture;
#[path = "../../wreckordbox-sidecar/tests/fingerprint_support/store.rs"]
mod store;
use rbl_library::{LibraryCore, NoHost};
use std::sync::Arc;
use wreckordbox_service::{Config, Service};
#[test]
fn foundation_stems_render_current_without_upgrading_or_emitting() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir(&library).unwrap();
    let location = rbl_db::fixture::build(&library, rbl_db::fixture::Shape::default()).unwrap();
    let mut tracks = vec![];
    {
        let mut w =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
        w.disable_automatic_backups();
        for format in fixture::FORMATS {
            let p = fixture::write(dir.path(), format);
            tracks.push((format, w.import_file(&p).unwrap()));
        }
    }
    let sidecar = dir.path().join("sidecar");
    store::seed(&sidecar, &tracks);
    std::env::set_var(wreckordbox_service::stems::SIDECAR_DIR_ENV, &sidecar);
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    let core = Arc::new(LibraryCore::new(config.recovery_dir.clone()));
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
    drop(db);
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let service = Service::host(&location, &config, &core, &host).unwrap();
    let mut watcher = wreckordbox_service::Client::connect(&config.socket_path(&location)).unwrap();
    watcher.subscribe().unwrap();
    let before = store::snapshot(&sidecar);
    let library_before = store::snapshot(&library);
    let golden = store::golden();
    for (format, id) in &tracks {
        let handed = service.current_stems_for_render(id).unwrap();
        assert_eq!(
            handed.fact_id,
            golden[format]["stem_fact"].as_i64().unwrap()
        );
        assert!(handed.audio_unchanged().unwrap());
    }
    assert!(watcher
        .next_event(std::time::Duration::from_millis(150))
        .unwrap()
        .is_none());
    assert_eq!(store::snapshot(&sidecar), before);
    assert_eq!(store::snapshot(&library), library_before);
}
