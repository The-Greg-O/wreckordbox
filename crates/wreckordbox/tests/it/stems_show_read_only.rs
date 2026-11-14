//! `wreckordbox stems show` only reads (wbx-t3p): asking about a track's
//! Stems neither makes a Sidecar nor changes one. Staleness is worked out
//! for the answer from the track's audio as it is now, never stored by the
//! question. Checked byte for byte on the Sidecar's folder, through the real
//! binary against a service on a throwaway Library, with the separation
//! engine's fake backend.
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
use serde_json::{json, Value};

/// A 16-bit stereo 44.1 kHz WAV file of `frames` sample frames, every
/// sample `value`.
fn write_audio(path: &Path, frames: u32, value: i16) {
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
    for _ in 0..frames * 2 {
        out.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}

fn audio(sandbox: &Sandbox) -> PathBuf {
    let dir = sandbox.path().join("audio");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("Night Bus.wav")
}

fn import(sandbox: &Sandbox) -> String {
    let path = audio(sandbox);
    write_audio(&path, 4_410, 0);
    let mut id = String::new();
    sandbox.edit(|writer| id = writer.import_file(&path).unwrap());
    id
}

/// Every file and folder under `root` with its bytes; empty when `root`
/// does not exist.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
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

/// The Sidecar's folder is byte for byte as it was; names what changed.
fn unchanged(sandbox: &Sandbox, before: &BTreeMap<PathBuf, Vec<u8>>) {
    let after = snapshot(&sandbox.sidecar_dir());
    let changed: Vec<&PathBuf> = before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        // SQLite's shared-memory index of an open WAL connection: rebuilt
        // from the database and its journal, it records which readers are
        // reading, and any read through the writer's connection touches it.
        // Not stored data.
        .filter(|path| !path.to_string_lossy().ends_with("-shm"))
        .collect();
    assert!(
        changed.is_empty(),
        "asking changed the Sidecar: {changed:?}"
    );
}

fn json_ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.json()
}

#[test]
fn stems_show_on_a_library_with_no_sidecar_makes_none() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox);
    // The service is up and answering before the question is asked.
    sandbox.status();
    assert!(!sandbox.sidecar_dir().join("sidecar.db").exists());
    let before = snapshot(&sandbox.sidecar_dir());

    let shown = json_ok(&sandbox, &["stems", "show", &track]);

    assert_eq!(shown["stems"], Value::Null);
    assert_eq!(shown["previous"], json!([]));
    assert!(
        !sandbox.sidecar_dir().join("sidecar.db").exists(),
        "asking made a Sidecar"
    );
    unchanged(&sandbox, &before);
    // The text form too.
    let text = sandbox.cli(&["stems", "show", &track]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(!sandbox.sidecar_dir().join("sidecar.db").exists());
}

#[test]
fn stems_show_of_stems_gone_stale_answers_stale_and_leaves_the_sidecar_as_it_was() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox);
    let made = json_ok(&sandbox, &["stems", "separate", &track]);
    let fact = made["stems"]["fact_id"].clone();
    // The same file, the same length, different audio: nothing has
    // written the Sidecar since.
    write_audio(&audio(&sandbox), 4_410, 1_000);
    let before = snapshot(&sandbox.sidecar_dir());

    let shown = json_ok(&sandbox, &["stems", "show", &track]);

    // Worked out for the answer...
    assert_eq!(shown["stems"], Value::Null, "{shown}");
    assert_eq!(shown["previous"][0]["fact_id"], fact, "{shown}");
    assert_eq!(shown["previous"][0]["stale"], true, "{shown}");
    // ...and not stored by it.
    unchanged(&sandbox, &before);
    let again = json_ok(&sandbox, &["stems", "show", &track]);
    assert_eq!(again, shown);
    unchanged(&sandbox, &before);

    // The audio coming back makes them current again, still by reading.
    write_audio(&audio(&sandbox), 4_410, 0);
    let back = json_ok(&sandbox, &["stems", "show", &track]);
    assert_eq!(back["stems"]["fact_id"], fact, "{back}");
    assert_eq!(back["stems"]["stale"], false, "{back}");
    assert_eq!(back["previous"], json!([]), "{back}");
    unchanged(&sandbox, &before);

    // The write that finds the change still stores it.
    write_audio(&audio(&sandbox), 4_410, 1_000);
    let separated = json_ok(&sandbox, &["stems", "separate", &track]);
    assert_eq!(separated["marked_stale"], json!([fact]));
}

#[test]
fn stems_marked_stale_by_a_write_read_current_again_once_their_audio_is_back() {
    let sandbox = Sandbox::new();
    let track = import(&sandbox);
    let made = json_ok(&sandbox, &["stems", "separate", &track]);
    let fact = made["stems"]["fact_id"].clone();
    write_audio(&audio(&sandbox), 4_410, 1_000);
    // A write about the track stores the mark (every Fact producer does).
    let recorded = json_ok(
        &sandbox,
        &["facts", "record", &track, "mood", "Dark", "--source", "dj"],
    );
    assert_eq!(recorded["marked_stale"], json!([fact]), "{recorded}");
    write_audio(&audio(&sandbox), 4_410, 0);
    let before = snapshot(&sandbox.sidecar_dir());

    let shown = json_ok(&sandbox, &["stems", "show", &track]);
    let status = sandbox
        .client()
        .call::<Value>("stems.status", json!({ "id": track }))
        .unwrap();

    assert_eq!(shown["stems"]["fact_id"], fact, "{shown}");
    assert_eq!(shown["stems"]["stale"], false, "{shown}");
    assert_eq!(status["state"], "current", "{status}");
    assert_eq!(status["fact_id"], fact, "{status}");
    unchanged(&sandbox, &before);
}
