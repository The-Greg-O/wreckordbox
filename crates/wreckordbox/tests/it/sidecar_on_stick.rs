//! Long background work and a Sidecar kept on a Stick (wbx-45.14): stem
//! separation and Intensity measurement are refused outright, even with
//! the Stick unlocked, before anything is written. A temporary folder
//! carrying the stand-in marker is the Stick; the separation engine is the
//! fake one, and it is never asked.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::Sandbox;

/// A short 16-bit stereo 44.1 kHz WAV file of silence.
fn write_audio(path: &Path) {
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
    out.resize(out.len() + data_len as usize, 0);
    std::fs::write(path, out).unwrap();
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.insert(path.clone(), Vec::new());
                todo.push(path);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// A sandbox whose Sidecar folder is a Stick, unlocked by name, with one
/// imported track; the service is started (and has opened its Sidecar).
fn sidecar_on_an_unlocked_stick() -> (Sandbox, String) {
    let sandbox = Sandbox::new();
    let sidecar = sandbox.sidecar_dir();
    std::fs::create_dir_all(&sidecar).unwrap();
    std::fs::write(
        sidecar.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let audio = sandbox.path().join("audio/Long Track.wav");
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    write_audio(&audio);
    let mut id = String::new();
    sandbox.edit(|writer| id = writer.import_file(&audio).unwrap());
    let name = sidecar.file_name().unwrap().to_str().unwrap().to_owned();
    let unlocked = sandbox.cli(&[
        "stick",
        "unlock",
        sidecar.to_str().unwrap(),
        "--name",
        &name,
    ]);
    assert!(unlocked.status.success(), "{}", unlocked.stderr);
    let recorded = sandbox.cli(&[
        "--json", "facts", "record", &id, "mood", "Dark", "--source", "dj",
    ]);
    assert!(
        recorded.status.success(),
        "a short Sidecar write on the unlocked Stick works: {}",
        recorded.stderr
    );
    (sandbox, id)
}

fn refused(sandbox: &Sandbox, args: &[&str]) {
    let before = snapshot(&sandbox.sidecar_dir());
    let run = sandbox.cli(args);
    assert!(!run.status.success(), "{args:?}: {}", run.stdout);
    let error = run.error_json();
    assert_eq!(
        error["error"]["code"], "sidecar_on_stick",
        "{args:?}: {error}"
    );
    let message = error["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("Sidecar") && message.contains("Nothing was written"),
        "{message}"
    );
    assert_eq!(
        snapshot(&sandbox.sidecar_dir()),
        before,
        "{args:?}: the Sidecar on the Stick is untouched"
    );
}

#[test]
fn a_separation_is_refused_for_a_sidecar_on_a_stick_even_unlocked() {
    let (sandbox, track) = sidecar_on_an_unlocked_stick();
    refused(&sandbox, &["--json", "stems", "separate", &track]);
    assert_eq!(
        sandbox.engine_runs(),
        0,
        "the separation engine was never asked"
    );
}

#[test]
fn a_measurement_is_refused_for_a_sidecar_on_a_stick_even_unlocked() {
    let (sandbox, track) = sidecar_on_an_unlocked_stick();
    refused(&sandbox, &["--json", "intensity", "measure", &track]);
}
