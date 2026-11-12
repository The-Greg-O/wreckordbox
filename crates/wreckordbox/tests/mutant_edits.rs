//! Mutant edits through the real CLI and fixture service.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;
use common::Sandbox;
use serde_json::{json, Value};

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = s.cli(&all);
    assert!(out.status.success(), "{args:?}: {}", out.stderr);
    out.json()
}
fn refused(s: &Sandbox, args: &[&str]) -> String {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = s.cli(&all);
    assert!(!out.status.success(), "{args:?}: {}", out.stdout);
    out.error_json()["error"]["code"].as_str().unwrap().into()
}
fn make(s: &Sandbox, playlist: &str) -> String {
    ok(
        s,
        &[
            "mutants",
            "create",
            "--playlist",
            playlist,
            "--position",
            "1",
            "--source",
            "dj",
        ],
    )["mutant"]["id"]
        .to_string()
}
fn edit(s: &Sandbox, id: &str, edits: Value) -> Value {
    ok(
        s,
        &[
            "mutants",
            "edit",
            id,
            "--edits",
            &edits.to_string(),
            "--source",
            "agent:tester",
        ],
    )
}
#[test]
fn edits_combine_derive_and_clear_without_changing_a_protected_parent() {
    let s = Sandbox::new();
    s.describe_track(2, "Parent", "Artist", "8A", 3);
    s.edit(|w| {
        w.add_cue("10002", 1, 250).unwrap();
        w.add_cue("10002", 0, 500).unwrap();
    });
    let playlist = s.create_playlist("Set", &[2]);
    s.protect(true);
    let parent = ok(&s, &["tracks", "show", "10002"]);
    let id = make(&s, &playlist);
    let initial = ok(&s, &["mutants", "show", &id]);
    assert_eq!(
        initial["mutant"]["derived"]["cues"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        refused(&s, &["tracks", "rate", "10002", "5"]),
        "library_protected"
    );
    let edited = edit(&s, &id, json!({"key_shift":1,"tempo":1.1}));
    assert_eq!(
        edited["mutant"]["edits"],
        json!({"key_shift":1,"tempo":1.1})
    );
    assert_eq!(edited["mutant"]["derived"]["key"], "3A");
    assert!((edited["mutant"]["derived"]["bpm"].as_f64().unwrap() - 128.02 * 1.1).abs() < 1e-8);
    assert_eq!(
        edit(&s, &id, json!({"cues":[]}))["mutant"]["derived"]["cues"],
        json!([])
    );
    for field in ["key_shift", "tempo", "cues"] {
        ok(&s, &["mutants", "clear", &id, field, "--source", "dj"]);
    }
    let cleared = ok(&s, &["mutants", "show", &id]);
    assert_eq!(cleared["mutant"]["edits"], json!({}));
    assert_eq!(cleared["mutant"]["derived"], initial["mutant"]["derived"]);
    assert_eq!(ok(&s, &["tracks", "show", "10002"]), parent);
}

#[test]
fn reads_do_not_create_a_sidecar() {
    let s = Sandbox::new();
    assert_eq!(ok(&s, &["mutants", "list"])["total"], 0);
    assert_eq!(refused(&s, &["mutants", "show", "99"]), "mutant_not_found");
    assert!(!s.sidecar_dir().exists());
}

#[test]
fn refusals_are_atomic_and_have_stable_codes() {
    let s = Sandbox::new();
    let playlist = s.create_playlist("Set", &[2]);
    let id = make(&s, &playlist);
    let before = ok(&s, &["mutants", "show", &id]);
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    while watcher
        .next_event(std::time::Duration::from_millis(100))
        .unwrap()
        .is_some()
    {}
    for (edits, code) in [
        (json!({"key_shift":7}), "mutant_key_shift_limit"),
        (json!({"key_shift":-7}), "mutant_key_shift_limit"),
        (json!({"key_shift":1000000}), "mutant_key_shift_limit"),
        (json!({"tempo":0.83999}), "mutant_tempo_limit"),
        (json!({"tempo":1.16001}), "mutant_tempo_limit"),
        (
            json!({"key_shift":2,"stems":["vocals"]}),
            "mutant_stems_missing",
        ),
        (json!({"stems":[]}), "bad_request"),
        (json!({"stems":["vocals","vocals"]}), "bad_request"),
        (json!({"energy":5}), "bad_request"),
        (
            json!({"cues":[{"kind":1,"position_ms":500,"out_ms":400}]}),
            "mutant_invalid_cues",
        ),
    ] {
        assert_eq!(
            refused(
                &s,
                &[
                    "mutants",
                    "edit",
                    &id,
                    "--edits",
                    &edits.to_string(),
                    "--source",
                    "dj"
                ]
            ),
            code
        );
        assert_eq!(ok(&s, &["mutants", "show", &id]), before);
    }
    assert_eq!(
        watcher
            .next_event(std::time::Duration::from_millis(200))
            .unwrap(),
        None
    );
    let changed = edit(&s, &id, json!({"key_shift":-6,"tempo":0.84}));
    assert_eq!(
        changed["run"]["id"].as_i64().unwrap(),
        before["mutant"]["run"].as_i64().unwrap() + 1
    );
    edit(&s, &id, json!({"key_shift":6,"tempo":1.16}));
    // Identity settings remove the corresponding edits even without Stems.
    assert_eq!(
        edit(
            &s,
            &id,
            json!({"key_shift":0,"tempo":1.0,"stems":["drums","vocals","bass","melody"]})
        )["mutant"]["edits"],
        json!({})
    );
}

fn audio_mutant(s: &Sandbox, name: &str) -> (String, String) {
    let track = s.import_track(name, 1);
    wav44(&s.path().join("audio").join(name), 44_100);
    let mut playlist = String::new();
    s.edit(|w| {
        playlist = w.create_playlist("Set", "root").unwrap();
        w.add_tracks(&playlist, std::slice::from_ref(&track))
            .unwrap();
    });
    let id = make(s, &playlist);
    (track, id)
}
fn flip_audio(s: &Sandbox, track: &str) {
    let path = ok(s, &["tracks", "show", track])["location"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[44] ^= 7;
    std::fs::write(path, bytes).unwrap();
}
#[test]
fn stems_are_verified_and_stale_edits_remain_clearable() {
    let s = Sandbox::new();
    let (track, id) = audio_mutant(&s, "Parent.wav");
    assert_eq!(
        refused(
            &s,
            &[
                "mutants",
                "edit",
                &id,
                "--edits",
                r#"{"stems":["vocals"]}"#,
                "--source",
                "dj"
            ]
        ),
        "mutant_stems_missing"
    );
    ok(&s, &["stems", "separate", &track]);
    s.protect(true);
    let result = edit(
        &s,
        &id,
        json!({"stems":["melody","bass","drums"],"key_shift":1,"tempo":1.1}),
    );
    assert_stretcher_readiness(&s.path().join("audio/Parent.wav"), &result, 1, 1.1);
    assert!(result["mutant"]["derived"]["description"]
        .as_str()
        .unwrap()
        .starts_with("no vocals, +1 st"));
    flip_audio(&s, &track);
    let before = ok(&s, &["mutants", "show", &id, "--check-audio"]);
    assert_eq!(
        before["mutant"]["renderability"]["reason"]["code"],
        "mutant_stems_stale"
    );
    assert_eq!(
        refused(
            &s,
            &[
                "mutants",
                "edit",
                &id,
                "--edits",
                r#"{"stems":["drums"]}"#,
                "--source",
                "dj"
            ]
        ),
        "mutant_stems_stale"
    );
    assert_eq!(ok(&s, &["mutants", "show", &id]), before);
    assert_eq!(
        edit(&s, &id, json!({"key_shift":2}))["mutant"]["renderability"]["reason"]["code"],
        "mutant_stems_stale"
    );
    let cleared = ok(&s, &["mutants", "clear", &id, "stems", "--source", "dj"]);
    assert_stretcher_readiness(&s.path().join("audio/Parent.wav"), &cleared, 2, 1.1);
    assert_eq!(
        cleared["mutant"]["edits"],
        json!({"key_shift":2,"tempo":1.1})
    );
    assert_eq!(s.engine_runs(), 1);
}

#[test]
fn edits_notify_a_second_client_and_preserve_run_provenance() {
    let s = Sandbox::new();
    let playlist = s.create_playlist("Set", &[2]);
    let id = make(&s, &playlist);
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:preparer",
            "--description",
            "Set edits",
        ],
    )["id"]
        .to_string();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    while watcher
        .next_event(std::time::Duration::from_millis(100))
        .unwrap()
        .is_some()
    {}
    let edited = ok(
        &s,
        &[
            "mutants",
            "edit",
            &id,
            "--edits",
            r#"{"key_shift":1,"tempo":1.1}"#,
            "--run",
            &run,
        ],
    );
    let event = watcher
        .next_event(std::time::Duration::from_secs(3))
        .unwrap()
        .unwrap();
    assert_eq!(event.event, "sidecar_changed");
    assert_eq!(event.data["action"], "edited");
    assert_eq!(event.data["kind"], "mutant");
    assert_eq!(
        watcher
            .next_event(std::time::Duration::from_millis(200))
            .unwrap(),
        None
    );
    assert_eq!(edited["created_run"], false);
    assert_eq!(edited["run"]["finished_at"], Value::Null);
    let history = ok(&s, &["mutants", "show", &id])["history"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1]["edits"], json!({"key_shift":1,"tempo":1.1}));
    assert_eq!(
        history[1]["source"],
        json!({"kind":"agent","name":"preparer"})
    );
    assert_eq!(history[1]["run"].to_string(), run);
    ok(&s, &["runs", "finish", &run]);
    assert_eq!(
        refused(&s, &["mutants", "clear", &id, "tempo", "--run", &run]),
        "run_finished"
    );
}

#[test]
fn lossy_parent_stem_edits_are_stored_but_explicitly_not_renderable() {
    for extension in ["m4a", "mp4", "m4b", "bin"] {
        let s = Sandbox::new();
        let track = s.import_track("Parent.wav", 1);
        let path = s.path().join("audio").join(format!("Parent.{extension}"));
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../rbl-audio/tests/fixtures/tone-faststart.m4a"
            ),
            &path,
        )
        .unwrap();
        let mut playlist = String::new();
        s.edit(|w| {
            w.relocate(&track, &path).unwrap();
            playlist = w.create_playlist("Set", "root").unwrap();
            w.add_tracks(&playlist, std::slice::from_ref(&track))
                .unwrap();
        });
        seed_stems(&s, &track, &path, 44_100);
        let id = make(&s, &playlist);
        let changed = edit(&s, &id, json!({"stems":["vocals"]}));
        assert_eq!(changed["mutant"]["edits"]["stems"], json!(["vocals"]));
        assert_eq!(
            changed["mutant"]["renderability"]["reason"]["code"], "stem_alignment_unknown",
            "{changed}"
        );
        assert_eq!(
            ok(&s, &["mutants", "show", &id])["mutant"],
            changed["mutant"]
        );
        assert!(s.cli(&["mutants", "show", &id]).stdout.contains("wbx-49.1"));
        assert_eq!(
            ok(&s, &["mutants", "clear", &id, "stems", "--source", "dj"])["mutant"]
                ["renderability"]["renderable"],
            true
        );
    }
}

fn wav44(path: &std::path::Path, frames: u32) {
    let data = frames * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&44100u32.to_le_bytes());
    out.extend_from_slice(&176400u32.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    out.resize(44 + data as usize, 0);
    std::fs::write(path, out).unwrap();
}
fn seed_stems(s: &Sandbox, track: &str, parent: &std::path::Path, frames: u32) {
    use wreckordbox_sidecar::stems::{NewStems, Role};
    use wreckordbox_sidecar::{Sidecar, Source, SourceKind};
    let store = Sidecar::open(&s.sidecar_dir()).unwrap();
    let source = Source::new(SourceKind::Tool, "fixture");
    let run = store.begin_run(&source, "Test Stems").unwrap();
    let workspace = store.workspace(run).unwrap();
    let files: Vec<_> = Role::ALL
        .into_iter()
        .map(|role| {
            let path = workspace.join(format!("{role}.wav"));
            wav44(&path, frames);
            (role, path)
        })
        .collect();
    let fingerprint = wreckordbox_sidecar::fingerprint_file(parent).unwrap();
    store
        .record_stems(&NewStems {
            track_id: track,
            fingerprint: &fingerprint,
            parent_samples: u64::from(frames),
            sample_rate: 44100,
            files: &files,
            source: &source,
            produced_by: json!({"engine":"test"}),
            run,
        })
        .unwrap();
    store.finish_run(run, "recorded").unwrap();
}

#[test]
fn review_renderer_length_and_capability_refusals_are_reported() {
    let s = Sandbox::new();
    let track = s.import_track("Parent.wav", 1);
    let path = s.path().join("audio/Parent.wav");
    wav44(&path, 44100);
    let mut playlist = String::new();
    s.edit(|w| {
        playlist = w.create_playlist("Set", "root").unwrap();
        w.add_tracks(&playlist, std::slice::from_ref(&track))
            .unwrap();
    });
    seed_stems(&s, &track, &path, 44101);
    let id = make(&s, &playlist);
    assert_eq!(
        edit(&s, &id, json!({"stems":["vocals"]}))["mutant"]["renderability"]["reason"]["code"],
        "stems_do_not_fit_parent"
    );
    ok(&s, &["mutants", "clear", &id, "stems", "--source", "dj"]);
    assert_stretcher_readiness(&path, &edit(&s, &id, json!({"key_shift":1})), 1, 1.0);
    ok(
        &s,
        &["mutants", "clear", &id, "key_shift", "--source", "dj"],
    );
    assert_stretcher_readiness(&path, &edit(&s, &id, json!({"tempo":1.1})), 0, 1.1);
}

fn assert_stretcher_readiness(path: &std::path::Path, response: &Value, key_shift: i8, tempo: f64) {
    use wreckordbox_render::{preflight, Edits, Kept, RenderError};
    // Cargo unifies features when the renderer and CLI suites run together.
    // The CLI must describe whichever capabilities that binary actually has.
    let expected = match preflight(path, Kept::ALL, None, Edits { key_shift, tempo }) {
        Ok(()) => json!({"renderable":true,"reason":null}),
        Err(error) => {
            let code = match error {
                RenderError::KeyShiftUnavailable => "key_shift_unavailable",
                RenderError::TempoUnavailable => "tempo_unavailable",
                other => panic!("unexpected fixture preflight error: {other}"),
            };
            json!({"renderable":false,"reason":{"code":code,"message":error.to_string()}})
        }
    };
    assert_eq!(response["mutant"]["renderability"], expected);
}

fn stop(s: &Sandbox) {
    ok(s, &["stop"]);
    for _ in 0..100 {
        if !s.sidecar_dir().join("sidecar.db-wal").exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("Sidecar did not close");
}
fn files(dir: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut result = std::collections::BTreeMap::new();
    if dir.exists() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                result.extend(files(&path));
            } else {
                result.insert(path.clone(), std::fs::read(path).unwrap());
            }
        }
    }
    result
}
#[test]
fn reads_of_saved_and_stale_mutants_do_not_write_or_tidy_anything() {
    let s = Sandbox::new();
    let (track, id) = audio_mutant(&s, "Parent.wav");
    ok(&s, &["stems", "separate", &track]);
    edit(&s, &id, json!({"stems":["vocals"]}));
    flip_audio(&s, &track);
    stop(&s);
    // Startup Ledger recovery intentionally opens an existing writer. Start
    // before placing the saved store back to exercise the command's unopened
    // snapshot path independently of that pre-existing startup behavior.
    let saved = s.path().join("saved-sidecar");
    std::fs::rename(s.sidecar_dir(), &saved).unwrap();
    s.status();
    std::fs::rename(saved, s.sidecar_dir()).unwrap();
    let work = s.sidecar_dir().join("work/keep.txt");
    std::fs::create_dir_all(work.parent().unwrap()).unwrap();
    std::fs::write(&work, b"do not tidy on read").unwrap();
    let before = files(&s.sidecar_dir());
    let show = ok(&s, &["mutants", "show", &id]);
    assert_eq!(
        show["mutant"]["renderability"]["reason"]["code"],
        "mutant_audio_unverified"
    );
    assert_eq!(ok(&s, &["mutants", "list"])["mutants"][0], show["mutant"]);
    assert_eq!(
        ok(&s, &["mutants", "show", &id, "--check-audio"])["mutant"]["renderability"]["reason"]
            ["code"],
        "mutant_stems_stale"
    );
    let after = files(&s.sidecar_dir());
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
    for (path, bytes) in &before {
        assert!(after[path] == *bytes, "read changed {}", path.display());
    }
}

#[test]
fn a_mutant_inherits_live_parent_metadata_and_cues_until_overridden() {
    let s = Sandbox::new();
    s.describe_track(2, "Before", "Old Artist", "8A", 1);
    let tag = s.create_my_tag("Warm");
    let playlist = s.create_playlist("Set", &[2]);
    let id = make(&s, &playlist);
    edit(&s, &id, json!({"key_shift":1}));
    ok(&s, &["tracks", "rate", "10002", "5"]);
    ok(&s, &["mytags", "assign", &tag, "10002"]);
    let inherited = ok(&s, &["mutants", "show", &id])["mutant"]["inherited"].clone();
    assert_eq!(inherited["energy"], 5);
    assert!(inherited["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["tag"] == "Warm"));
    assert_eq!(
        inherited["my_tags"],
        ok(&s, &["tracks", "show", "10002"])["my_tags"]
    );
    stop(&s);
    s.describe_track(2, "After", "New Artist", "9A", 4);
    s.edit(|w| {
        w.add_cue("10002", 2, 750).unwrap();
    });
    let after = ok(&s, &["mutants", "show", &id]);
    assert_eq!(after["mutant"]["parent"]["title"], "After");
    assert_eq!(after["mutant"]["inherited"]["artist"], "New Artist");
    assert_eq!(after["mutant"]["inherited"]["energy"], 4);
    assert_eq!(after["mutant"]["derived"]["key"], "4A");
    assert_eq!(after["mutant"]["derived"]["grid_parent"], "10002");
    assert_eq!(after["mutant"]["derived"]["cues"][0]["position_ms"], 750);
    let cue = json!({"kind":2,"position_ms":900,"comment":"Mix here"});
    let changed = edit(&s, &id, json!({"cues":[cue]}));
    assert_eq!(
        changed["mutant"]["derived"]["description"],
        "+1 st, cue B moved"
    );
    stop(&s);
    s.edit(|w| {
        w.add_cue("10002", 3, 1200).unwrap();
    });
    assert_eq!(
        ok(&s, &["mutants", "show", &id])["mutant"]["derived"]["cues"],
        changed["mutant"]["derived"]["cues"]
    );
    let entry = after["mutant"]["entry_id"].as_str().unwrap();
    assert_eq!(
        ok(
            &s,
            &["mutants", "clear", "--entry", entry, "cues", "--source", "dj"]
        )["mutant"]["derived"]["cues"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn concurrent_patches_preserve_both_edits() {
    let s = Sandbox::new();
    let playlist = s.create_playlist("Set", &[2]);
    let id = make(&s, &playlist);
    std::thread::scope(|scope| {
        let first = scope.spawn(|| edit(&s, &id, json!({"key_shift":2})));
        let second = scope.spawn(|| edit(&s, &id, json!({"tempo":1.05})));
        first.join().unwrap();
        second.join().unwrap();
    });
    let shown = ok(&s, &["mutants", "show", &id]);
    assert_eq!(
        shown["mutant"]["edits"],
        json!({"key_shift":2,"tempo":1.05})
    );
    assert_eq!(shown["history"].as_array().unwrap().len(), 3);
}

#[test]
fn an_edit_that_cannot_commit_changes_nothing_and_announces_nothing() {
    let s = Sandbox::new();
    let playlist = s.create_playlist("Set", &[2]);
    let id = make(&s, &playlist);
    let before = ok(&s, &["mutants", "show", &id]);
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    while watcher
        .next_event(std::time::Duration::from_millis(100))
        .unwrap()
        .is_some()
    {}
    let db = rusqlite::Connection::open(s.sidecar_dir().join("sidecar.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_ending BEFORE UPDATE OF finished_at ON run BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert_eq!(
        refused(
            &s,
            &[
                "mutants",
                "edit",
                &id,
                "--edits",
                r#"{"key_shift":2}"#,
                "--source",
                "dj"
            ]
        ),
        "sidecar_failed"
    );
    assert_eq!(ok(&s, &["mutants", "show", &id]), before);
    assert_eq!(
        watcher
            .next_event(std::time::Duration::from_millis(200))
            .unwrap(),
        None
    );
    db.execute_batch("DROP TRIGGER refuse_ending").unwrap();
    assert_eq!(
        edit(&s, &id, json!({"key_shift":2}))["mutant"]["edits"],
        json!({"key_shift":2})
    );
}

#[test]
fn damaged_stored_edits_are_reported_instead_of_inventing_an_unedited_mutant() {
    let s = Sandbox::new();
    let playlist = s.create_playlist("Set", &[2]);
    let id = make(&s, &playlist);
    let db = rusqlite::Connection::open(s.sidecar_dir().join("sidecar.db")).unwrap();
    db.execute(
        "UPDATE mutant_change SET edits = '{broken' WHERE mutant_id = ?1",
        [&id],
    )
    .unwrap();
    let shown = ok(&s, &["mutants", "show", &id]);
    assert_eq!(shown["mutant"]["edits"], "{broken");
    assert_eq!(shown["mutant"]["derived"], Value::Null);
    assert_eq!(
        shown["mutant"]["renderability"]["reason"]["code"],
        "mutant_edits_invalid"
    );
    assert_eq!(ok(&s, &["mutants", "list"])["mutants"][0], shown["mutant"]);
    assert_eq!(
        ok(&s, &["mutants", "remove", &id, "--source", "dj"])["mutant"]["removed"],
        true
    );
}

#[test]
fn review_two_clients_each_make_150_edits_lists_and_shows_without_failure() {
    let s = Sandbox::new();
    let playlist = s.create_playlist("Set", &[2, 3]);
    let id = make(&s, &playlist).parse::<i64>().unwrap();
    std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..2).map(|who| scope.spawn({ let s=&s; move || {
            let mut client = s.client();
            let mut failures = Vec::new();
            for i in 0..150 {
                for (command, args) in [
                    ("mutants.edit", json!({"id":id,"edits":{"key_shift":i%5},"source":{"kind":"dj"}})),
                    ("mutants.list", json!({})),
                    ("mutants.get", json!({"id":id})),
                ] {
                    if let Err(error) = client.call::<Value>(command, args) { failures.push(format!("{command}: {error:?}")); }
                }
            }
            assert_eq!(failures, Vec::<String>::new(), "client {who}");
        }})).collect();
        for job in jobs {
            job.join().unwrap();
        }
    });
}

#[test]
fn review_outdated_edits_are_isolated_and_repairable() {
    let s = Sandbox::new();
    let playlist = s.create_playlist("Set", &[2]);
    let neighbour = s.create_playlist("Healthy", &[3]);
    let id = make(&s, &playlist);
    let healthy = make(&s, &neighbour);
    let healthy_before = ok(&s, &["mutants", "show", &healthy]);
    let db = rusqlite::Connection::open(s.sidecar_dir().join("sidecar.db")).unwrap();
    let break_it = || {
        db.execute(
            "UPDATE mutant_change SET edits = '{\"tempo\":1.3}' WHERE mutant_id = ?1",
            [&id],
        )
        .unwrap()
    };
    break_it();
    let listed = ok(&s, &["mutants", "list"]);
    assert_eq!(listed["total"], 2);
    assert_eq!(listed["mutants"][1], healthy_before["mutant"]);
    assert_eq!(
        listed["mutants"][0]["renderability"]["reason"]["code"],
        "mutant_edits_invalid"
    );
    assert_eq!(listed["mutants"][0]["derived"], Value::Null);
    assert_eq!(
        ok(&s, &["mutants", "show", &id])["mutant"]["edits"],
        json!({"tempo":1.3})
    );
    assert_eq!(
        ok(&s, &["mutants", "clear", &id, "tempo", "--source", "dj"])["mutant"]["edits"],
        json!({})
    );
    break_it();
    assert_eq!(
        edit(&s, &id, json!({"tempo":1.1}))["mutant"]["edits"],
        json!({"tempo":1.1})
    );
    break_it();
    assert_eq!(
        ok(&s, &["mutants", "remove", &id, "--source", "dj"])["mutant"]["removed"],
        true
    );
}

#[test]
fn review_noop_edits_leave_no_run_change_or_event() {
    let s = Sandbox::new();
    let playlist = s.create_playlist("Set", &[2]);
    let id = make(&s, &playlist);
    let before = ok(&s, &["mutants", "show", &id]);
    let db = rusqlite::Connection::open(s.sidecar_dir().join("sidecar.db")).unwrap();
    let counts = || {
        db.query_row(
            "SELECT (SELECT count(*) FROM run), (SELECT count(*) FROM mutant_change)",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap()
    };
    let old_counts = counts();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    while watcher
        .next_event(std::time::Duration::from_millis(100))
        .unwrap()
        .is_some()
    {}
    for edits in [
        json!({}),
        json!({"key_shift":null}),
        json!({"key_shift":0,"tempo":1.0}),
        json!({"stems":["vocals","melody","bass","drums"]}),
    ] {
        let answer = edit(&s, &id, edits);
        assert_eq!(answer["changed"], false);
        assert_eq!(answer["run"], Value::Null);
    }
    let cleared = ok(&s, &["mutants", "clear", &id, "tempo", "--source", "dj"]);
    assert_eq!(cleared["changed"], false);
    assert_eq!(cleared["run"], Value::Null);
    assert_eq!(ok(&s, &["mutants", "show", &id]), before);
    assert_eq!(counts(), old_counts);
    assert_eq!(
        watcher
            .next_event(std::time::Duration::from_millis(100))
            .unwrap(),
        None
    );
    edit(&s, &id, json!({"key_shift":2}));
    let changed_counts = counts();
    let changed_history = ok(&s, &["mutants", "show", &id]);
    assert_eq!(edit(&s, &id, json!({"key_shift":2}))["changed"], false);
    assert_eq!(counts(), changed_counts);
    assert_eq!(ok(&s, &["mutants", "show", &id]), changed_history);
}

#[test]
fn review_key_recognition_matches_the_window() {
    for (stored, expected) in [
        ("A minor", json!("Bbm")),
        ("Amin", json!("Bbm")),
        ("AM", json!("Bbm")),
        ("1m", json!("8m")),
        ("1d", json!("8d")),
        ("BB", json!("B")),
        ("♭", Value::Null),
    ] {
        let s = Sandbox::new();
        s.describe_track(2, "Parent", "Artist", stored, 3);
        let playlist = s.create_playlist("Set", &[2]);
        let id = make(&s, &playlist);
        assert_eq!(
            edit(&s, &id, json!({"key_shift":1}))["mutant"]["derived"]["key"],
            expected,
            "{stored}"
        );
    }
}

#[test]
fn review_cues_respect_length_and_round_trip_real_parent_rows() {
    let s = Sandbox::new();
    s.edit(|w| {
        w.add_cue("10002",1,100).unwrap(); w.add_cue("10002",1,200).unwrap();
        for (id,kind,at) in [("legacy-loop",4,300),("legacy-kind",25,400)] {
            w.library().connection().execute("INSERT INTO djmdCue (ID,ContentID,Kind,InMsec,OutMsec,Color,ColorTableIndex,Comment,rb_local_deleted,created_at,updated_at) VALUES (?1,'10002',?2,?3,0,0,0,'',0,'2026-01-01','2026-01-01')",rusqlite::params![id,kind,at]).unwrap();
        }
    });
    let playlist = s.create_playlist("Set", &[2]);
    let id = make(&s, &playlist);
    let parent = ok(&s, &["mutants", "show", &id])["mutant"]["derived"]["cues"].clone();
    assert_eq!(
        edit(&s, &id, json!({"cues":parent}))["mutant"]["derived"]["cues"],
        parent
    );
    for cue in [
        json!({"kind":0,"position_ms":999999999}),
        json!({"kind":0,"position_ms":1,"out_ms":999999999}),
        json!({"kind":2,"position_ms":100,"memory_colour":3}),
    ] {
        assert_eq!(
            refused(
                &s,
                &[
                    "mutants",
                    "edit",
                    &id,
                    "--edits",
                    &json!({"cues":[cue]}).to_string(),
                    "--source",
                    "dj"
                ]
            ),
            "mutant_invalid_cues"
        );
    }
}

#[test]
#[ignore = "explicit 500-Mutant latency measurement"]
#[allow(clippy::print_stdout)] // This explicitly invoked benchmark reports its measurements.
fn review_benchmark_listing_500_mutants() {
    let s = Sandbox::new();
    let track = s.import_track("Parent.wav", 1);
    let mut playlist = String::new();
    s.edit(|w| {
        playlist = w.create_playlist("Large set", "root").unwrap();
        w.add_tracks_allowing(&playlist, &vec![track.clone(); 500], true)
            .unwrap();
    });
    ok(&s, &["stems", "separate", &track]);
    let mut client = s.client();
    for position in 1..=500 {
        let made: Value = client
            .call(
                "mutants.create",
                json!({"playlist":playlist,"position":position,"source":{"kind":"dj"}}),
            )
            .unwrap();
        let _:Value=client.call("mutants.edit",json!({"id":made["mutant"]["id"],"edits":{"stems":["vocals"]},"source":{"kind":"dj"}})).unwrap();
    }
    for iteration in 0..3 {
        let start = std::time::Instant::now();
        let response: Value = client.call("mutants.list", json!({})).unwrap();
        assert_eq!(response["total"], 500);
        println!("500 Mutants, one shared 1-second WAV parent, warm service, iteration {iteration}: {:.3} ms",start.elapsed().as_secs_f64()*1000.0);
    }
}

#[test]
fn review_reads_do_not_scan_audio_and_plain_edits_do_not_fingerprint() {
    let s = Sandbox::new();
    let track = s.import_track("Long.wav", 60);
    let mut playlist = String::new();
    s.edit(|w| {
        playlist = w.create_playlist("Set", "root").unwrap();
        w.add_tracks(&playlist, std::slice::from_ref(&track))
            .unwrap();
    });
    let id = make(&s, &playlist);
    ok(&s, &["stems", "separate", &track]);
    edit(&s, &id, json!({"stems":["vocals"]}));
    let pid = s.status()["pid"].as_u64().unwrap();
    let bytes = || {
        std::fs::read_to_string(format!("/proc/{pid}/io"))
            .unwrap()
            .lines()
            .find_map(|l| {
                l.strip_prefix("rchar: ")
                    .map(|v| v.trim().parse::<u64>().unwrap())
            })
            .unwrap()
    };
    let before = bytes();
    ok(&s, &["mutants", "show", &id]);
    ok(&s, &["mutants", "list"]);
    edit(&s, &id, json!({"key_shift":1}));
    assert!(
        bytes() - before < 2_000_000,
        "read {} bytes for two reads and a key edit",
        bytes() - before
    );
}

#[test]
fn review_wav_contents_with_mp3_name_are_renderable() {
    let s = Sandbox::new();
    let track = s.import_track("Parent.wav", 1);
    wav44(&s.path().join("audio/Parent.wav"), 44100);
    let path = s.path().join("audio/Parent.mp3");
    std::fs::rename(s.path().join("audio/Parent.wav"), &path).unwrap();
    let mut playlist = String::new();
    s.edit(|w| {
        w.relocate(&track, &path).unwrap();
        playlist = w.create_playlist("Set", "root").unwrap();
        w.add_tracks(&playlist, std::slice::from_ref(&track))
            .unwrap();
    });
    let id = make(&s, &playlist);
    ok(&s, &["stems", "separate", &track]);
    assert_eq!(
        edit(&s, &id, json!({"stems":["vocals"]}))["mutant"]["renderability"]["renderable"],
        true
    );
}
