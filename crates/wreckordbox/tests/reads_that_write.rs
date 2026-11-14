//! Byte-level regressions for reads; real CLI, isolated Library, second client.
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

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    if !root.exists() {
        return out;
    }
    out.insert(root.to_path_buf(), vec![]);
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.to_string_lossy().ends_with("-shm") {
                continue;
            }
            if path.is_dir() {
                out.insert(path.clone(), vec![]);
                todo.push(path);
            } else {
                out.insert(path.clone(), std::fs::read(path).unwrap());
            }
        }
    }
    out
}
fn unchanged(before: &BTreeMap<PathBuf, Vec<u8>>, after: &BTreeMap<PathBuf, Vec<u8>>) {
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .collect();
    assert!(changed.is_empty(), "read changed bytes: {changed:?}");
}
fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend(args);
    let r = s.cli(&all);
    assert!(r.status.success(), "{args:?}: {}", r.stderr);
    r.json()
}
fn watcher(s: &Sandbox) -> wreckordbox_service::Client {
    let mut c = s.client();
    c.subscribe().unwrap();
    // An identified Sidecar event orders all setup events on its queue.
    // An absent or locked Sidecar has no setup writes to drain.
    if s.sidecar_dir().join("sidecar.db").exists()
        && !s
            .sidecar_dir()
            .join(wreckordbox_stick_protection::STAND_IN_MARKER)
            .exists()
    {
        let recorded = ok(
            s,
            &[
                "facts",
                "record",
                "10000",
                "vocals",
                "barrier",
                "--source",
                "tool:read-barrier",
                "--confidence",
                "1",
            ],
        );
        let id = recorded["facts"][0]["id"].as_i64().unwrap();
        loop {
            let event = c
                .next_event(Duration::from_secs(5))
                .unwrap()
                .expect("setup Fact event");
            if event.event == "sidecar_changed" && event.data["fact_id"] == id {
                break;
            }
        }
    }
    c
}
fn query(s: &Sandbox, read: impl FnOnce() -> Value) -> Value {
    let mut c = watcher(s);
    let sidecar = snapshot(&s.sidecar_dir());
    let library = snapshot(s.master_db().parent().unwrap());
    let result = read();
    let event = c.next_event(Duration::from_millis(150)).unwrap();
    // Check both effects before asserting so the failure identifies both.
    let after = snapshot(&s.sidecar_dir());
    eprintln!("event after read: {event:?}");
    unchanged(&sidecar, &after);
    unchanged(&library, &snapshot(s.master_db().parent().unwrap()));
    assert!(event.is_none(), "read emitted {event:?}");
    result
}
#[test]
fn intensity_show_without_sidecar_creates_none() {
    let s = Sandbox::new();
    let id = s.import_track("signal.wav", 1);
    s.status();
    let shown = query(&s, || ok(&s, &["intensity", "show", &id]));
    assert!(shown["intensity"].is_null());
    assert!(!s.sidecar_dir().exists());
}
#[test]
fn intensity_show_changed_audio_is_stale_without_bytes_or_events() {
    let s = Sandbox::new();
    let id = s.import_track("signal.wav", 1);
    let measured = ok(&s, &["intensity", "measure", &id]);
    let fact = measured["tracks"][0]["fact_id"].clone();
    let path = s.path().join("audio/signal.wav");
    let original = std::fs::read(&path).unwrap();
    let mut changed = original.clone();
    changed[44] ^= 1;
    std::fs::write(&path, changed).unwrap();
    let shown = query(&s, || ok(&s, &["intensity", "show", &id]));
    assert!(shown["intensity"].is_null());
    assert_eq!(shown["previous"][0]["fact_id"], fact);
    assert_eq!(shown["previous"][0]["stale"], true);
    // Only the explicit write marks it; returning audio reads current without clearing the mark.
    let next = ok(&s, &["intensity", "measure", &id]);
    assert_eq!(next["tracks"][0]["marked_stale"], json!([fact]));
    std::fs::write(path, original).unwrap();
    let shown = query(&s, || ok(&s, &["intensity", "show", &id]));
    // The old value is superseded by the second measurement, never resurrected.
    assert!(shown["intensity"].is_null());
    let reused = ok(&s, &["intensity", "measure", &id]);
    assert_eq!(reused["reused"], 1);
    assert_eq!(reused["decoded"], 0);
}
#[test]
fn facts_kinds_without_sidecar_creates_none() {
    let s = Sandbox::new();
    s.status();
    query(&s, || ok(&s, &["facts", "kinds"]));
}
#[test]
fn decisions_list_without_sidecar_creates_none() {
    let s = Sandbox::new();
    s.status();
    query(&s, || ok(&s, &["decisions", "list"]));
}
#[test]
fn checked_facts_only_compute_staleness() {
    let s = Sandbox::new();
    let id = s.import_track("signal.wav", 1);
    ok(&s, &["intensity", "measure", &id]);
    let path = s.path().join("audio/signal.wav");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[44] ^= 1;
    std::fs::write(path, bytes).unwrap();
    let shown = query(&s, || ok(&s, &["facts", "show", &id, "--check-audio"]));
    assert_eq!(shown["stale"].as_array().unwrap().len(), 1);
    assert_eq!(shown["reattached"], json!([]));
}

fn sweep(s: &Sandbox, id: &str, playlist: &str, tag: &str, populated: bool) {
    let stick = s.path().join("spare");
    std::fs::create_dir_all(&stick).unwrap();
    std::fs::write(
        stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let calls = vec![
        ("status", json!({})),
        ("protection.get", json!({})),
        ("tracks.list", json!({})),
        ("tracks.get", json!({"id":id})),
        ("tracks.query", json!({})),
        ("mytags.list", json!({})),
        ("mytags.of_track", json!({"id":id})),
        ("mytags.tracks", json!({"id":tag})),
        ("playlists.tree", json!({})),
        ("playlists.tracks", json!({"id":playlist})),
        ("history.list", json!({})),
        ("history.tracks", json!({"id":"missing"})),
        ("intake.list", json!({})),
        ("intake.problems", json!({})),
        ("intake.counts", json!({})),
        ("intake.colours", json!({})),
        ("intake.tag_panel", json!({"id":id})),
        ("stems.show", json!({"id":id})),
        ("stems.status", json!({"id":id})),
        ("intensity.show", json!({"id":id})),
        ("facts.kinds", json!({})),
        ("facts.of_track", json!({"id":id})),
        (
            "facts.of_track",
            json!({"id":id,"check_audio":true,"all":true}),
        ),
        ("facts.standing", json!({"id":id,"kind":"intensity"})),
        ("runs.list", json!({})),
        ("runs.show", json!({"id":1})),
        ("runs.active", json!({})),
        ("decisions.list", json!({})),
        ("decisions.get", json!({"id":1})),
        ("mutants.list", json!({})),
        ("mutants.list", json!({"check_audio":true})),
        ("mutants.get", json!({"id":1})),
        ("mutants.get", json!({"id":1,"check_audio":true})),
        ("review.waiting", json!({})),
        ("review.rejected", json!({})),
        ("projection.barriers", json!({})),
        ("projection.plan", json!({})),
        ("schema.check", json!({})),
        ("schema.convert", json!({"dry_run":true})),
        ("sticks.list", json!({})),
        (
            "sticks.dry_run",
            json!({"stick":stick,"playlists":[playlist]}),
        ),
    ];
    let mut failures = vec![];
    let mut watcher = watcher(s);
    for (name, args) in calls {
        let sidecar = snapshot(&s.sidecar_dir());
        let library = snapshot(s.master_db().parent().unwrap());
        let stick_before = snapshot(&stick);
        let answer = s.client().call::<Value>(name, args.clone());
        if let Err(e) = &answer {
            let allowed = match name {
                "history.tracks" => "history_not_found",
                "runs.show" if !populated => "run_not_found",
                "decisions.get" if !populated => "decision_not_found",
                "mutants.get" if !populated => "mutant_not_found",
                "schema.check" => "tag_schema_missing",
                _ => "no error expected",
            };
            if e.code != allowed {
                failures.push(format!("{name} {args}: unexpected {e:?}"));
            }
        }
        let event = watcher.next_event(Duration::from_millis(50)).unwrap();
        if event.is_some() {
            failures.push(format!("{name}: event {event:?}"));
        }
        for (before, after) in [
            (&sidecar, snapshot(&s.sidecar_dir())),
            (&library, snapshot(s.master_db().parent().unwrap())),
            (&stick_before, snapshot(&stick)),
        ] {
            let changed: Vec<_> = before
                .keys()
                .chain(after.keys())
                .filter(|p| before.get(*p) != after.get(*p))
                .collect();
            if !changed.is_empty() {
                failures.push(format!("{name}: wrote {changed:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "query audit failures:\n{}",
        failures.join("\n")
    );
}
#[test]
fn every_query_with_absent_sidecar_is_read_only() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    let id = s.import_track("signal.wav", 1);
    let playlist = s.create_playlist("Read audit", &[0]);
    let tag = s.create_my_tag("Audit");
    s.status();
    sweep(&s, &id, &playlist, &tag, false);
}
#[test]
fn every_query_with_existing_sidecar_is_read_only() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    let id = s.import_track("signal.wav", 1);
    let playlist = s.create_playlist("Read audit", &[0]);
    let tag = s.create_my_tag("Audit");
    ok(&s, &["intensity", "measure", &id]);
    ok(&s, &["stems", "separate", &id]);
    s.client()
        .call::<Value>(
            "decisions.create",
            json!({"question_type":"choice","question":"Which?","candidates":["A","B"]}),
        )
        .unwrap();
    s.client()
        .call::<Value>(
            "mutants.create",
            json!({"playlist":playlist,"position":1,"source":{"kind":"dj","name":"dj"}}),
        )
        .unwrap();
    sweep(&s, &id, &playlist, &tag, true);
}

fn give_beat_grid(sandbox: &Sandbox, track: &str, bpm: f64, first_downbeat: f64, seconds: f64) {
    let mut beats = Vec::new();
    let mut at = first_downbeat;
    let mut number = 0_u16;
    while at < seconds {
        beats.push(rbl_anlz::Beat {
            beat_number: number % 4 + 1,
            tempo_x100: (bpm * 100.0) as u16,
            time_ms: (at * 1000.0).round() as u32,
        });
        number += 1;
        at = first_downbeat + f64::from(number) * 60.0 / bpm;
    }
    let relative = format!("/PIONEER/USBANLZ/{track}/ANLZ0000.DAT");
    let file = rbl_anlz::resolve(&sandbox.location().share_root, &relative);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let bytes = rbl_anlz::AnlzBuilder::new()
        .path("/audio/track.wav")
        .beat_grid(&beats)
        .finish();
    std::fs::write(&file, bytes).unwrap();
    sandbox.edit(|writer| {
        let changed = writer
            .library()
            .connection()
            .execute(
                "UPDATE djmdContent SET AnalysisDataPath = ?1 WHERE ID = ?2",
                (relative.as_str(), track),
            )
            .unwrap();
        assert_eq!(changed, 1);
    });
}

fn stop(s: &Sandbox) {
    let pid = s.status()["pid"].as_u64().unwrap();
    ok(s, &["stop"]);
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while common::alive(pid) {
        assert!(
            std::time::Instant::now() < until,
            "service still owns Library"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn intensity_alignment_is_computed_and_the_write_records_it() {
    let s = Sandbox::new();
    let id = s.import_track("signal.wav", 5);
    let measured = ok(&s, &["intensity", "measure", &id]);
    let fact = measured["tracks"][0]["fact_id"].clone();
    stop(&s);
    give_beat_grid(&s, &id, 120.0, 0.5, 5.0);
    s.status();
    let shown = query(&s, || ok(&s, &["intensity", "show", &id]));
    assert_eq!(shown["intensity"]["fact_id"], fact);
    assert_eq!(
        shown["intensity"]["value"]["bars"]["starts_seconds"],
        json!([0.5, 2.5, 4.5])
    );
    let stored = ok(&s, &["facts", "show", &id, "--all"]);
    assert!(stored["facts"][0]["value"]["bars"].is_null());
    let reused = ok(&s, &["intensity", "measure", &id]);
    assert_eq!(reused["decoded"], 0);
    assert_eq!(reused["tracks"][0]["reused_from"], fact);
    assert_eq!(reused["tracks"][0]["bars"], 3);
}
#[test]
fn intensity_on_locked_stick_reads_immutable_snapshot() {
    let s = Sandbox::new();
    let id = s.import_track("signal.wav", 1);
    let measured = ok(&s, &["intensity", "measure", &id]);
    stop(&s);
    std::fs::write(
        s.sidecar_dir()
            .join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    s.status();
    let shown = query(&s, || ok(&s, &["intensity", "show", &id]));
    assert_eq!(
        shown["intensity"]["fact_id"],
        measured["tracks"][0]["fact_id"]
    );
}
#[test]
fn queries_refuse_unopened_journal_without_recovering_or_tidying() {
    let s = Sandbox::new();
    let id = s.import_track("signal.wav", 1);
    s.status();
    drop(wreckordbox_sidecar::Sidecar::open(&s.sidecar_dir()).unwrap());
    std::fs::write(s.sidecar_dir().join("sidecar.db-wal"), b"unfinished").unwrap();
    std::fs::create_dir_all(s.sidecar_dir().join("work")).unwrap();
    std::fs::write(s.sidecar_dir().join("work/keep"), b"untouched").unwrap();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    for (name, args) in [
        ("intensity.show", json!({"id":id})),
        ("facts.of_track", json!({"id":id,"check_audio":true})),
        ("decisions.list", json!({})),
        ("decisions.get", json!({"id":1})),
    ] {
        let before = snapshot(&s.sidecar_dir());
        let error = s.client().call::<Value>(name, args).unwrap_err();
        assert_eq!(error.code, "sidecar_failed", "{name}: {error:?}");
        unchanged(&before, &snapshot(&s.sidecar_dir()));
        assert!(watcher
            .next_event(Duration::from_millis(50))
            .unwrap()
            .is_none());
    }
}
#[test]
fn backup_list_reads_without_creating_storage() {
    let s = Sandbox::new();
    s.status();
    let before = snapshot(&s.state_dir());
    query(&s, || ok(&s, &["backup", "list"]));
    unchanged(&before, &snapshot(&s.state_dir()));
}
