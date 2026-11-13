//! Standing Facts through the real CLI and a fixture service.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;
use common::Sandbox;
use serde_json::{json, Value};
use std::time::Duration;

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut command = vec!["--json"];
    command.extend_from_slice(args);
    let result = s.cli(&command);
    assert!(result.status.success(), "{args:?}: {}", result.stderr);
    result.json()
}
fn record(s: &Sandbox, source: &str, value: &str, confidence: &str) -> Value {
    ok(
        s,
        &[
            "facts",
            "record",
            "10003",
            "energy",
            value,
            "--source",
            source,
            "--confidence",
            confidence,
        ],
    )["facts"][0]
        .clone()
}
fn standing(s: &Sandbox) -> Value {
    ok(s, &["facts", "standing", "10003", "energy"])
}
#[test]
fn the_dj_stands_over_every_source_and_the_corrected_answers_remain_readable() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let agent = record(&s, "agent:tagger", "5", "1");
    let dj = record(&s, "dj", "2", "0.1");
    let tool = record(&s, "tool:meter", "4", "1");
    let web = record(&s, "web:https://example.test/track", "3", "1");
    for _ in 0..4 {
        assert_eq!(
            watcher
                .next_event(Duration::from_secs(2))
                .unwrap()
                .unwrap()
                .event,
            "sidecar_changed"
        );
    }
    let before = ok(&s, &["runs", "list"]);
    let result = standing(&s);
    assert_eq!(result["standing"]["id"], dj["id"]);
    assert_eq!(result["reason"]["code"], "dj");
    for old in [&agent, &tool, &web] {
        let kept = result["superseded"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == old["id"])
            .unwrap();
        assert_eq!(kept["value"], old["value"]);
        assert_eq!(kept["source"], old["source"]);
        assert_eq!(kept["superseded_by"], dj["id"]);
    }
    assert_eq!(ok(&s, &["runs", "list"]), before);
    assert!(watcher
        .next_event(Duration::from_millis(200))
        .unwrap()
        .is_none());
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 0);
}
#[test]
fn source_rules_reload_and_confidence_breaks_ties_without_rewriting_facts() {
    let s = Sandbox::new();
    let trusted = record(&s, "agent:trusted", "2", "0.2");
    let certain = record(&s, "tool:meter", "4", "0.99");
    assert_eq!(standing(&s)["standing"]["id"], certain["id"]);
    assert_eq!(standing(&s)["reason"]["code"], "confidence");
    let file = s.sidecar_dir().join("source-precedence.toml");
    std::fs::write(
        &file,
        "[sources]\n\"agent:trusted\" = 10\n\"tool:meter\" = -2\n",
    )
    .unwrap();
    let result = standing(&s);
    assert_eq!(result["standing"]["id"], trusted["id"]);
    assert_eq!(result["reason"]["code"], "source_precedence");
    assert_eq!(result["rules_file"], file.to_str().unwrap());
    std::fs::write(&file, "[sources]\n\"tool:meter\" = 20\n").unwrap();
    assert_eq!(standing(&s)["standing"]["id"], certain["id"]);
    let dj = record(&s, "dj", "1", "0");
    assert_eq!(standing(&s)["standing"]["id"], dj["id"]);
    std::fs::write(&file, "[sources]\n\"agent:trusted\" = \"oops\"\n").unwrap();
    let refused = s.cli(&["--json", "facts", "standing", "10003", "energy"]);
    assert!(!refused.status.success());
    assert_eq!(
        refused.error_json()["error"]["code"],
        "source_precedence_invalid"
    );
    assert!(refused.error_json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("source-precedence.toml"));
}
#[test]
fn querying_an_empty_sidecar_creates_nothing_and_is_readable_with_protection_on() {
    let s = Sandbox::new();
    s.protect(true);
    let result = standing(&s);
    assert_eq!(result["standing"], Value::Null);
    assert_eq!(result["reason"]["code"], "no_eligible_fact");
    assert_eq!(result["superseded"], json!([]));
    assert!(!s.sidecar_dir().exists());
    let refused = s.cli(&["--json", "tracks", "--source", "dj", "rate", "10003", "4"]);
    assert!(!refused.status.success());
    assert_eq!(refused.error_json()["error"]["code"], "library_protected");
    assert!(!s.sidecar_dir().exists());
}

#[test]
fn measurements_are_verified_without_marking_stale_or_reattaching_on_a_read() {
    let s = Sandbox::new();
    let track = s.import_track("Meter.wav", 1);
    let audio = s.path().join("audio/Meter.wav");
    let original = std::fs::read(&audio).unwrap();
    let recorded = ok(
        &s,
        &[
            "facts",
            "record",
            &track,
            "intensity",
            r#"{"summary":0.7}"#,
            "--source",
            "tool:meter",
            "--confidence",
            "1",
        ],
    );
    let id = &recorded["facts"][0]["id"];
    let before = ok(&s, &["facts", "show", &track, "--all"]);
    let runs = ok(&s, &["runs", "list"]);
    let query = ["facts", "standing", &track, "intensity"];
    assert_eq!(ok(&s, &query)["standing"]["id"], *id);
    let mut changed = original.clone();
    changed[44] ^= 0x55;
    std::fs::write(&audio, &changed).unwrap();
    let result = ok(&s, &query);
    assert_eq!(result["standing"], Value::Null);
    assert_eq!(result["stale"][0]["id"], *id);
    assert_eq!(result["stale"][0]["stale"], true);
    assert_eq!(ok(&s, &["facts", "show", &track, "--all"]), before);
    assert_eq!(ok(&s, &["runs", "list"]), runs);
    std::fs::write(&audio, &original).unwrap();
    assert_eq!(ok(&s, &query)["standing"]["id"], *id);
    std::fs::remove_file(&audio).unwrap();
    let result = s.cli(&["--json", "facts", "standing", &track, "intensity"]);
    assert!(!result.status.success());
    assert_eq!(result.error_json()["error"]["code"], "track_file_missing");
    assert_eq!(ok(&s, &["facts", "show", &track, "--all"]), before);
}

#[test]
fn same_source_replacements_stay_archived_and_ties_are_deterministic() {
    let s = Sandbox::new();
    let first = record(&s, "agent:one", "5", "1");
    let replaced = record(&s, "agent:one", "2", "0.2");
    assert_eq!(standing(&s)["standing"]["id"], replaced["id"]);
    let second = record(&s, "agent:two", "3", "0.2");
    let result = standing(&s);
    assert_eq!(result["standing"]["id"], second["id"]);
    assert_eq!(result["reason"]["code"], "latest_fact");
    let archived = result["superseded"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == first["id"])
        .unwrap();
    assert_eq!(archived["superseded_by"], replaced["id"]);
    assert_eq!(archived["confidence"], 1.0);
    let raw_before = ok(&s, &["facts", "show", "10003", "--all"]);
    let text = s.cli(&["facts", "standing", "10003", "energy"]);
    assert!(text.status.success(), "{}", text.stderr);
    assert!(text.stdout.contains(&format!(
        "Standing Fact {}: 3 (agent:two, confidence 0.2).",
        second["id"]
    )));
    assert!(text.stdout.contains("Source precedence and confidence tie"));
    assert!(text.stdout.contains("Superseded Fact"));
    assert_eq!(ok(&s, &["facts", "show", "10003", "--all"]), raw_before);
}

#[test]
fn invalid_or_unreadable_rules_do_not_fall_back_and_cannot_demote_the_dj() {
    let s = Sandbox::new();
    record(&s, "dj", "2", "1");
    let file = s.sidecar_dir().join("source-precedence.toml");
    for contents in [
        "[sources]\ndj = -10\n",
        "[sources]\n\"agent:\" = 10\n",
        "[sources]\n\"Agent:tagger\" = 10\n",
        "[source]\n\"agent:tagger\" = 10\n",
        "[sources]\n\"agent:tagger\" = 1.5\n",
        "not TOML [",
    ] {
        std::fs::write(&file, contents).unwrap();
        let result = s.cli(&["--json", "facts", "standing", "10003", "energy"]);
        assert!(!result.status.success(), "{contents}");
        assert_eq!(
            result.error_json()["error"]["code"],
            "source_precedence_invalid"
        );
    }
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let result = s.cli(&["--json", "facts", "standing", "10003", "energy"]);
    assert!(!result.status.success());
    assert_eq!(
        result.error_json()["error"]["code"],
        "source_precedence_invalid"
    );
}

#[test]
fn a_snapshot_query_never_migrates_or_tidies_an_existing_sidecar() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    // Install an old store after the service starts, so its lazy writer is
    // unopened. The read must report the problem and leave every byte alone.
    std::fs::create_dir_all(s.sidecar_dir().join("work/keep")).unwrap();
    let sentinel = s.sidecar_dir().join("work/keep/sentinel");
    std::fs::write(&sentinel, "do not tidy a read").unwrap();
    let file = s.sidecar_dir().join("sidecar.db");
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT); INSERT INTO meta VALUES ('schema_version', '1');").unwrap();
    drop(db);
    let before = std::fs::read(&file).unwrap();
    let result = s.cli(&["--json", "facts", "standing", "10003", "energy"]);
    assert!(!result.status.success());
    assert_eq!(result.error_json()["error"]["code"], "sidecar_failed");
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert_eq!(
        std::fs::read_to_string(&sentinel).unwrap(),
        "do not tidy a read"
    );
    assert!(!s.sidecar_dir().join("sidecar.db-wal").exists());
}

#[test]
fn a_read_does_not_reattach_but_resolves_the_mapping_after_a_producer_does() {
    let s = Sandbox::new();
    let old = s.import_track("Readd.wav", 1);
    let mut service = s.serve();
    let dj = ok(
        &s,
        &["facts", "record", &old, "energy", "2", "--source", "dj"],
    )["facts"][0]
        .clone();
    service.kill().unwrap();
    service.wait().unwrap();
    let mut new = String::new();
    s.edit(|writer| {
        writer.delete_track(&old).unwrap();
        new = writer
            .import_file(&s.path().join("audio/Readd.wav"))
            .unwrap();
    });
    assert_ne!(new, old);
    let before = ok(&s, &["runs", "list"]);
    assert_eq!(
        ok(&s, &["facts", "standing", &new, "energy"])["standing"],
        Value::Null
    );
    assert_eq!(ok(&s, &["runs", "list"]), before);
    // A genuine producer observes the current identity, then records.
    ok(
        &s,
        &[
            "facts",
            "record",
            &new,
            "energy",
            "5",
            "--source",
            "agent:tagger",
            "--confidence",
            "1",
        ],
    );
    let result = ok(&s, &["facts", "standing", &new, "energy"]);
    assert_eq!(result["standing"]["id"], dj["id"]);
    assert_eq!(result["standing"]["run"], dj["run"]);
    assert_eq!(result["standing"]["track_id"], new);
    assert_eq!(result["reason"]["code"], "dj");
}

#[test]
fn an_unreadable_store_is_not_reported_as_an_empty_answer() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    std::fs::create_dir_all(s.sidecar_dir().join("sidecar.db")).unwrap();
    let result = s.cli(&["--json", "facts", "standing", "10003", "energy"]);
    assert!(
        !result.status.success(),
        "an unusable store was silently treated as empty: {}",
        result.stdout
    );
    assert_eq!(result.error_json()["error"]["code"], "sidecar_failed");
}

#[test]
fn a_missing_rules_symlink_target_is_an_error_not_default_precedence() {
    let s = Sandbox::new();
    ok(&s, &["status"]);
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    let rules = s.sidecar_dir().join("source-precedence.toml");
    std::os::unix::fs::symlink("missing-rules.toml", &rules).unwrap();
    let result = s.cli(&["--json", "facts", "standing", "10003", "energy"]);
    assert!(
        !result.status.success(),
        "broken source rules were ignored: {}",
        result.stdout
    );
    assert_eq!(
        result.error_json()["error"]["code"],
        "source_precedence_invalid"
    );
    assert!(!s.sidecar_dir().join("sidecar.db").exists());
}
