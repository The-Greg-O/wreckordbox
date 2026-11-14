//! Versioned evidence through the real CLI, preview cache and fresh Stick load.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;
use common::Sandbox;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};
use wreckordbox_sidecar::fingerprint_file;
fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend(args);
    let r = s.cli(&all);
    assert!(r.status.success(), "{args:?}: {}", r.stderr);
    r.json()
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.to_string_lossy().ends_with("-shm") {
                continue;
            }
            if p.is_dir() {
                todo.push(p)
            } else {
                out.insert(p.clone(), std::fs::read(p).unwrap());
            }
        }
    }
    out
}
fn stop(s: &Sandbox) {
    let pid = s.status()["pid"].as_u64().unwrap();
    ok(s, &["stop"]);
    let end = std::time::Instant::now() + Duration::from_secs(10);
    while common::alive(pid) {
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn wav(path: &Path, format: u16, value: u32) {
    let mut b = b"RIFF".to_vec();
    let len = 44100u32 * 4;
    b.extend((36 + len).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(format.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(44100u32.to_le_bytes());
    b.extend((44100u32 * 4).to_le_bytes());
    b.extend(4u16.to_le_bytes());
    b.extend(32u16.to_le_bytes());
    b.extend(b"data");
    b.extend(len.to_le_bytes());
    for _ in 0..44100 {
        b.extend(value.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}
fn exercise(legacy: bool) {
    let mut s = Sandbox::new();
    s.idle_exit_secs = 120;
    let stick = s.path().join("SPARE");
    std::fs::create_dir(&stick).unwrap();
    for marker in [
        wreckordbox_stick_protection::STAND_IN_MARKER,
        wreckordbox_stick_protection::SPARE_MARKER,
    ] {
        std::fs::write(stick.join(marker), b"").unwrap();
    }
    s.sticks = vec![stick.clone()];
    let id = s.import_track("Track.wav", 1);
    let audio = s.path().join("audio/Track.wav");
    wav(&audio, 1, 0.25f32.to_bits());
    let fresh = fingerprint_file(&audio).unwrap();
    assert!(fresh.as_str().starts_with("a2:"));
    let mut playlist = String::new();
    s.edit(|w| {
        playlist = w.create_playlist("Set", "root").unwrap();
        w.add_tracks(&playlist, std::slice::from_ref(&id)).unwrap();
    });
    let made = ok(&s, &["stems", "separate", &id]);
    let stem = made["stems"]["fact_id"].as_i64().unwrap();
    ok(&s, &["intensity", "measure", &id]);
    stop(&s);
    let expected = if legacy {
        fresh.legacy()
    } else {
        fresh.clone()
    };
    if legacy {
        // Emulate a pre-upgrade store, preserving its original Fact payloads.
        let db = rusqlite::Connection::open(s.sidecar_dir().join("sidecar.db")).unwrap();
        db.execute(
            "UPDATE fact SET fingerprint=?1 WHERE track_id=?2",
            (expected.as_str(), &id),
        )
        .unwrap();
        db.execute(
            "UPDATE track_identity SET fingerprint=?1 WHERE track_id=?2",
            (expected.as_str(), &id),
        )
        .unwrap();
    }
    s.status();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let before = snapshot(&s.sidecar_dir());
    let library = snapshot(s.master_db().parent().unwrap());
    let stick_before = snapshot(&stick);
    let path = stick.to_str().unwrap();
    for _ in 0..2 {
        // Second dry run uses the in-memory preview fingerprint.
        assert_eq!(
            ok(&s, &["stems", "show", &id])["stems"]["fingerprint"],
            json!(expected)
        );
        assert!(!ok(&s, &["intensity", "show", &id])["intensity"].is_null());
        assert_eq!(
            ok(&s, &["facts", "show", &id, "--check-audio"])["stale"],
            json!([])
        );
        let dry = ok(&s, &["stick", "dry-run", path, &playlist]);
        assert_eq!(dry["changes"]["native_stems"].as_array().unwrap().len(), 1);
    }
    assert!(watcher
        .next_event(Duration::from_millis(150))
        .unwrap()
        .is_none());
    assert_eq!(snapshot(&s.sidecar_dir()), before);
    assert_eq!(snapshot(s.master_db().parent().unwrap()), library);
    assert_eq!(snapshot(&stick), stick_before);
    ok(&s, &["stick", "unlock", path]);
    let approved = ok(&s, &["stick", "dry-run", path, &playlist]);
    // Only format changes. Fresh load must apply the stored version's rule.
    let mtime = std::fs::metadata(&audio).unwrap().modified().unwrap();
    wav(&audio, 3, 0.25f32.to_bits());
    std::fs::File::options()
        .write(true)
        .open(&audio)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let now = fingerprint_file(&audio).unwrap();
    assert_ne!(now, fresh);
    assert_eq!(now.legacy(), fresh.legacy());
    let shown = ok(&s, &["stems", "show", &id]);
    assert_eq!(shown["stems"].is_null(), !legacy);
    assert_eq!(
        ok(&s, &["intensity", "show", &id])["intensity"].is_null(),
        !legacy
    );
    assert_eq!(
        ok(&s, &["facts", "show", &id, "--check-audio"])["stale"]
            .as_array()
            .unwrap()
            .len(),
        if legacy { 0 } else { 2 }
    );
    let dry = ok(&s, &["stick", "dry-run", path, &playlist]);
    assert_eq!(
        dry["changes"]["native_stems"].as_array().unwrap().len(),
        usize::from(legacy)
    );
    assert_eq!(snapshot(&s.sidecar_dir()), before);
    let load = s.cli(&[
        "--json",
        "stick",
        "load",
        path,
        &playlist,
        "--plan",
        approved["plan"].as_str().unwrap(),
    ]);
    if legacy {
        assert!(load.status.success(), "{}", load.stderr);
        assert_eq!(load.json()["native_stems"].as_array().unwrap().len(), 1);
        assert_eq!(
            snapshot(&s.sidecar_dir()),
            before,
            "load never upgrades stored evidence"
        );
        let reused = ok(&s, &["stems", "separate", &id]);
        assert_eq!(reused["reused"], true);
        assert_eq!(reused["stems"]["fact_id"], stem);
        assert_eq!(reused["stems"]["fingerprint"], json!(expected));
        assert_eq!(s.engine_runs(), 1);
        let reused = ok(&s, &["intensity", "measure", &id]);
        assert_eq!(reused["decoded"], 0);
        assert_eq!(reused["tracks"][0]["fingerprint"], json!(expected));
        // Reusing evidence on another track records a new Fact, but cannot
        // turn an old measurement into v2 evidence without decoding.
        stop(&s);
        let copy = s.path().join("audio/Copy.wav");
        std::fs::copy(&audio, &copy).unwrap();
        let mut other = String::new();
        s.edit(|w| other = w.import_file(&copy).unwrap());
        let copied = ok(&s, &["intensity", "measure", &other]);
        assert_eq!(copied["decoded"], 0);
        assert_eq!(copied["tracks"][0]["fingerprint"], json!(expected));
        let measured = ok(&s, &["intensity", "measure", &id, "--force"]);
        assert_eq!(measured["decoded"], 1);
        assert_eq!(measured["tracks"][0]["fingerprint"], json!(now));
        let all = ok(&s, &["facts", "show", &id, "--all"]);
        assert!(
            all["facts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["fingerprint"] == json!(expected)),
            "old evidence remains old"
        );
    } else {
        assert!(!load.status.success());
        assert_eq!(load.error_json()["error"]["code"], "stick_load_changed");
        assert_eq!(
            snapshot(&stick),
            stick_before,
            "refused load writes nothing"
        );
    }
}
#[test]
fn legacy_facts_preview_and_load_use_the_old_rule_without_upgrading_reads() {
    exercise(true)
}
#[test]
fn version_two_preview_and_load_refuse_same_packets_with_a_new_sample_format() {
    exercise(false)
}
