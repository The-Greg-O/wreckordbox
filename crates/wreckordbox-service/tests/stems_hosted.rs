//! Stems through a service a front end hosts on its own Library core, as
//! the window does: the Sidecar belongs to the service in that case too,
//! and the host hears of the separation without any change to the Library.
//!
//! One test, in a test binary of its own: the Sidecar's folder and the
//! engine's command come from the environment, which is the process's.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rbl_db::fixture::{self, Shape};
use rbl_library::{LibraryCore, NoHost};
use serde_json::json;
use wreckordbox_service::stems::{commands, events, SEPARATION_COMMAND_ENV, SIDECAR_DIR_ENV};
use wreckordbox_service::{Config, Service};

/// A silent 16-bit stereo 44.1 kHz WAV file of `frames` sample frames.
fn write_audio(path: &Path, frames: u32) {
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
    std::fs::write(path, out).unwrap();
}

#[test]
fn a_hosted_service_separates_into_its_sidecar_and_tells_the_host_without_a_library_change() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let location = fixture::build(
        &library,
        Shape {
            tracks: 3,
            ..Shape::default()
        },
    )
    .unwrap();
    let audio = dir.path().join("Night Bus.wav");
    write_audio(&audio, 2_205);
    let track = {
        let mut writer =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        writer.import_file(&audio).unwrap()
    };

    let sidecar = dir.path().join("sidecar");
    let fake =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../separation-engine/tests/fake_worker.py");
    std::env::set_var(SIDECAR_DIR_ENV, &sidecar);
    std::env::set_var(
        SEPARATION_COMMAND_ENV,
        json!(["python3", fake.to_str().unwrap()]).to_string(),
    );

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
    let generation = core.generation();
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let service = Service::host(&location, &config, &core, &host).unwrap();
    let heard = service.events();

    let separated = service
        .call(commands::STEMS_SEPARATE, json!({ "id": track }))
        .unwrap();

    assert_eq!(separated["reused"], false);
    assert_eq!(separated["stems"]["samples"], 2_205);
    let sidecar = sidecar.canonicalize().unwrap();
    for stem in separated["stems"]["stems"].as_array().unwrap() {
        let path = Path::new(stem["path"].as_str().unwrap());
        assert!(path.starts_with(&sidecar) && path.is_file(), "{stem}");
    }
    // The host is told of the progress and of the Sidecar's change, and of
    // nothing else: the Library did not change.
    let mut names = Vec::new();
    while let Ok(event) = heard.recv_timeout(Duration::from_millis(500)) {
        names.push(event.event);
    }
    assert!(
        names.contains(&events::STEMS_PROGRESS.to_owned()),
        "{names:?}"
    );
    assert_eq!(names.last().unwrap(), events::SIDECAR_CHANGED);
    assert!(!names.contains(&"library_changed".to_owned()), "{names:?}");
    assert_eq!(core.generation(), generation);

    let shown = service
        .call(commands::STEMS_SHOW, json!({ "id": track }))
        .unwrap();
    assert_eq!(shown["stems"], separated["stems"]);

    // An ordinary Fact read detects changed audio and tells the host once,
    // without a Library edit or an explicit check-audio request.
    let mut bytes = std::fs::read(&audio).unwrap();
    bytes[44] ^= 0x55;
    std::fs::write(&audio, bytes).unwrap();
    let facts = service
        .call("facts.of_track", json!({"id": track}))
        .unwrap();
    assert_eq!(facts["facts"], json!([]));
    assert_eq!(facts["stale"][0]["id"], separated["stems"]["fact_id"]);
    let changed = heard.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(changed.event, events::SIDECAR_CHANGED);
    assert_eq!(
        changed.data["stale"],
        json!([separated["stems"]["fact_id"]])
    );
    assert_eq!(core.generation(), generation);
    service
        .call("facts.of_track", json!({"id": track}))
        .unwrap();
    assert!(heard.recv_timeout(Duration::from_millis(150)).is_err());
}
