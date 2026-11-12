//! Intensity through a service a front end hosts, handed over to the next
//! one: a measuring caught between measuring and recording when the
//! service stops records nothing, and the service does not let go of the
//! Library until it is over.
//!
//! One test, in a test binary of its own: the Sidecar's folder and the
//! pause come from the environment, which is the process's.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rbl_db::fixture::{self, Shape};
use rbl_library::{LibraryCore, NoHost};
use serde_json::json;
use wreckordbox_service::intensity::{commands, PAUSE_ENV};
use wreckordbox_service::stems::SIDECAR_DIR_ENV;
use wreckordbox_service::{Config, Service};

/// Four seconds of a 16-bit mono 44.1 kHz WAV: a tone with a click every
/// half second.
fn write_audio(path: &Path) {
    let samples: Vec<i16> = (0..44_100 * 4)
        .map(|i| {
            let tone = (f64::from(i) * 2.0 * std::f64::consts::PI * 110.0 / 44_100.0).sin() * 0.3;
            let click = if i % 22_050 < 200 { 0.5 } else { 0.0 };
            ((tone + click) * 32_000.0) as i16
        })
        .collect();
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&44_100_u32.to_le_bytes());
    out.extend_from_slice(&(44_100_u32 * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

#[test]
fn a_hosted_service_that_stops_mid_measuring_records_nothing_after_and_hands_over_cleanly() {
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
    let audio = dir.path().join("Handover.wav");
    write_audio(&audio);
    let track = {
        let mut writer =
            rbl_db::write::Writer::open(location.clone(), dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        writer.import_file(&audio).unwrap()
    };
    let sidecar_dir = dir.path().join("sidecar");
    let pause = dir.path().join("pause");
    std::fs::create_dir_all(&pause).unwrap();
    std::env::set_var(SIDECAR_DIR_ENV, &sidecar_dir);
    std::env::set_var(PAUSE_ENV, &pause);
    std::env::set_var("RBXPORT_TEST", "1");

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
    let host: Arc<dyn rbl_library::Host + Send + Sync> = Arc::new(NoHost);
    let first = Arc::new(Service::host(&location, &config, &core, &host).unwrap());

    // Measured, and held just before it is recorded.
    let marker = pause.join(format!("{track}.commit"));
    std::fs::write(&marker, b"").unwrap();
    let measuring = {
        let first = Arc::clone(&first);
        let track = track.clone();
        std::thread::spawn(move || {
            first
                .call(commands::INTENSITY_MEASURE, json!({ "ids": [track] }))
                .unwrap()
        })
    };
    let reached = pause.join(format!("{track}.commit.reached"));
    let deadline = Instant::now() + Duration::from_secs(120);
    while !reached.exists() {
        assert!(Instant::now() < deadline, "the measuring never got to recording");
        std::thread::sleep(Duration::from_millis(10));
    }

    // The window closes: the service stops. It does not let go while the
    // measuring is still under way.
    let stopping = {
        let first = Arc::clone(&first);
        std::thread::spawn(move || first.stop())
    };
    std::thread::sleep(Duration::from_millis(400));
    assert!(!stopping.is_finished(), "the service stopped under a measuring");
    std::fs::remove_file(&marker).unwrap();
    stopping.join().unwrap();

    // The measuring saw it was cancelled and recorded nothing.
    let measured = measuring.join().unwrap();
    assert_eq!(measured["cancelled"], true, "{measured}");
    assert_eq!(measured["tracks"][0]["status"], "cancelled", "{measured}");
    let sidecar = wreckordbox_sidecar::Sidecar::open(&sidecar_dir).unwrap();
    assert!(sidecar.facts(&track, "intensity").unwrap().is_empty());

    // The next service has the Library and the Sidecar to itself, and
    // measures the track afresh.
    drop(first);
    let second = Service::host(&location, &config, &core, &host).unwrap();
    let again = second
        .call(commands::INTENSITY_MEASURE, json!({ "ids": [track] }))
        .unwrap();
    assert_eq!(again["measured"], 1, "{again}");
    assert_eq!(again["decoded"], 1, "{again}");
    let facts = sidecar.facts(&track, "intensity").unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, again["tracks"][0]["fact_id"]);
    second.stop();
}
