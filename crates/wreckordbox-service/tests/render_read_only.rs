//! The renderer's boundary: the service hands `wreckordbox_render` a Stem
//! set only when it is verified current for the track's audio as it is
//! now. Audio replaced by other audio of the same length, which the
//! renderer's own length check cannot see, is refused; the same audio
//! returning makes the Stems current again.
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

use rbl_db::fixture::{self, Shape};
use rbl_library::{LibraryCore, NoHost};
use serde_json::json;
use wreckordbox_service::native_stems::codes::NO_CURRENT_STEMS;
use wreckordbox_service::stems::{commands, SEPARATION_COMMAND_ENV, SIDECAR_DIR_ENV};
use wreckordbox_service::{Config, Service};

/// A 16-bit stereo 44.1 kHz WAV of `frames` frames of a ramp seeded by
/// `seed`: two seeds give two different audio files of the same length.
fn write_audio(path: &Path, frames: u32, seed: i16) {
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
    for i in 0..frames {
        let sample = ((i as i16).wrapping_mul(7)).wrapping_add(seed);
        out.extend_from_slice(&sample.to_le_bytes());
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

#[test]
fn render_lookup_never_writes_or_emits() {
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
    let audio = dir.path().join("Night Bus.wav");
    let frames = 4_410;
    write_audio(&audio, frames, 1);
    let original = std::fs::read(&audio).unwrap();
    let track = {
        let mut writer =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        writer.import_file(&audio).unwrap()
    };

    let fake =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../separation-engine/tests/fake_worker.py");
    std::env::set_var(SIDECAR_DIR_ENV, dir.path().join("sidecar"));
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
    drop(db);
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let service = Service::host(&location, &config, &core, &host).unwrap();
    let track = track.to_string();

    // No Stems yet: nothing to hand over.
    let none = service.current_stems_for_render(&track).unwrap_err();
    assert_eq!(none.code, NO_CURRENT_STEMS, "{none:?}");

    let mut watcher = wreckordbox_service::Client::connect(&config.socket_path(&location)).unwrap();
    watcher.subscribe().unwrap();
    let separated = service
        .call(commands::STEMS_SEPARATE, json!({ "id": track }))
        .unwrap();
    let handed = service.current_stems_for_render(&track).unwrap();
    assert_eq!(handed.stems.frames, u64::from(frames));
    assert_eq!(handed.fact_id, separated["stems"]["fact_id"]);
    assert_eq!(handed.parent.file_name(), audio.file_name());
    assert!(handed.audio_unchanged().unwrap());

    loop {
        let event = watcher
            .next_event(std::time::Duration::from_secs(5))
            .unwrap()
            .expect("separation event");
        if event.event == "sidecar_changed" && event.data["fact_id"] == handed.fact_id {
            break;
        }
    }
    write_audio(&audio, frames, 2);
    let before = snapshot(&dir.path().join("sidecar"));
    let stale = service.current_stems_for_render(&track).unwrap_err();
    assert_eq!(stale.code, NO_CURRENT_STEMS);
    let event = watcher
        .next_event(std::time::Duration::from_millis(150))
        .unwrap();
    eprintln!("render query event: {event:?}");
    let after = snapshot(&dir.path().join("sidecar"));
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .collect();
    assert!(changed.is_empty(), "render lookup wrote: {changed:?}");
    assert!(event.is_none(), "render lookup emitted {event:?}");
    std::fs::write(&audio, original).unwrap();
    assert_eq!(
        service.current_stems_for_render(&track).unwrap().fact_id,
        handed.fact_id
    );
    assert_eq!(snapshot(&dir.path().join("sidecar")), before);
}
fn snapshot(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut result = std::collections::BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
            } else if !path.to_string_lossy().ends_with("-shm") {
                result.insert(path.clone(), std::fs::read(path).unwrap());
            }
        }
    }
    result
}
