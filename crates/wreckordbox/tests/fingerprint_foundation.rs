//! Evidence written by foundation ec1d477d, verified through the real service.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;
#[path = "../../wreckordbox-sidecar/tests/fingerprint_support/mod.rs"]
mod fixture;
#[path = "../../wreckordbox-sidecar/tests/fingerprint_support/store.rs"]
mod store;
use common::Sandbox;
use serde_json::{json, Value};
fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend(args);
    let r = s.cli(&all);
    assert!(r.status.success(), "{args:?}: {}", r.stderr);
    r.json()
}
#[test]
fn foundation_evidence_stays_current_in_reads_preview_load_and_intensity_reuse() {
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
    let mut tracks = vec![];
    let mut playlist = String::new();
    s.edit(|w| {
        for format in fixture::FORMATS {
            let p = fixture::write(s.path(), format);
            tracks.push((format, w.import_file(&p).unwrap()));
        }
        playlist = w.create_playlist("Foundation fixture", "root").unwrap();
        w.add_tracks(
            &playlist,
            &tracks.iter().map(|(_, id)| id.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
    });
    store::seed(&s.sidecar_dir(), &tracks);
    s.status();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let before = store::snapshot(&s.sidecar_dir());
    let library = store::snapshot(s.master_db().parent().unwrap());
    let usb = store::snapshot(&stick);
    let path = stick.to_str().unwrap();
    let golden = store::golden();
    for (format, id) in &tracks {
        let status: Value = s.client().call("stems.status", json!({"id":id})).unwrap();
        assert_eq!(status["state"], "current", "{format}: {status}");
        assert_eq!(
            ok(&s, &["stems", "show", id])["stems"]["fingerprint"],
            golden[format]["a1"]
        );
        assert_eq!(
            ok(&s, &["facts", "show", id, "--check-audio"])["stale"],
            json!([])
        );
        assert!(!ok(&s, &["intensity", "show", id])["intensity"].is_null());
    }
    for _ in 0..2 {
        let dry = ok(&s, &["stick", "dry-run", path, &playlist]);
        assert_eq!(
            dry["changes"]["native_stems"].as_array().unwrap().len(),
            6,
            "{dry}"
        );
    }
    assert!(watcher
        .next_event(std::time::Duration::from_millis(150))
        .unwrap()
        .is_none());
    assert_eq!(store::snapshot(&s.sidecar_dir()), before);
    assert_eq!(store::snapshot(s.master_db().parent().unwrap()), library);
    assert_eq!(store::snapshot(&stick), usb);
    ok(&s, &["stick", "unlock", path]);
    let plan = ok(&s, &["stick", "dry-run", path, &playlist]);
    let load = ok(
        &s,
        &[
            "stick",
            "load",
            path,
            &playlist,
            "--plan",
            plan["plan"].as_str().unwrap(),
        ],
    );
    assert_eq!(load["native_stems"].as_array().unwrap().len(), 6, "{load}");
    assert_eq!(
        store::snapshot(&s.sidecar_dir()),
        before,
        "load must not upgrade old evidence"
    );
    for (format, id) in &tracks {
        let reused = ok(&s, &["intensity", "measure", id]);
        assert_eq!(reused["decoded"], 0, "{format}: {reused}");
        assert_eq!(reused["tracks"][0]["fingerprint"], golden[format]["a1"]);
    }
    assert_eq!(s.engine_runs(), 0);
}
