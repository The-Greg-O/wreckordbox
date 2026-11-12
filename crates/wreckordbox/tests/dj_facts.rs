//! Native hand edits and their standing Facts through the real CLI.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;
use common::Sandbox;
use serde_json::Value;
fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = s.cli(&all);
    assert!(out.status.success(), "{}", out.stderr);
    out.json()
}
#[test]
fn only_a_dj_edit_produces_a_fact_in_its_ledger_run() {
    let s = Sandbox::new();
    ok(
        &s,
        &["tracks", "rate", "10003", "5", "--source", "agent:tagger"],
    );
    assert!(ok(&s, &["facts", "standing", "10003", "energy"])["standing"].is_null());
    let edit = ok(&s, &["tracks", "rate", "10003", "2", "--source", "dj"]);
    let fact = ok(&s, &["facts", "standing", "10003", "energy"])["standing"].clone();
    assert_eq!(fact["value"], 2);
    assert_eq!(fact["run"], edit["run"]["id"]);
    assert_eq!(fact["source"]["kind"], "dj");
    assert_eq!(fact["reconciled"], false);
    let entries = ok(&s, &["runs", "show", &edit["run"]["id"].to_string()]);
    assert_eq!(fact["ledger_entry"], entries["entries"][0]["id"]);
}
#[test]
fn a_refused_dj_edit_leaves_no_fact_entry_or_intent() {
    let s = Sandbox::new();
    s.protect(true);
    assert!(!s
        .cli(&["--json", "tracks", "rate", "10003", "5", "--source", "dj"])
        .status
        .success());
    assert!(ok(&s, &["facts", "standing", "10003", "energy"])["standing"].is_null());
    assert_eq!(ok(&s, &["runs", "list"])["total"], 0);
    assert!(!s.sidecar_dir().join("sidecar.db").exists());
}

#[test]
fn prep_colour_and_schema_terms_follow_native_edits_and_clears() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(
        &s,
        &["intake", "colour", "10003", "Clean", "--source", "dj"],
    );
    assert_eq!(
        ok(&s, &["facts", "standing", "10003", "prep_colour"])["standing"]["value"],
        "Green"
    );
    ok(&s, &["schema", "convert"]);
    ok(
        &s,
        &[
            "mytags",
            "assign",
            rbl_db::fixture::MY_TAG_PEAK,
            "10003",
            "--source",
            "dj",
        ],
    );
    assert_eq!(
        ok(&s, &["facts", "standing", "10003", "situation"])["standing"]["value"],
        serde_json::json!(["peak"])
    );
    ok(
        &s,
        &[
            "mytags",
            "unassign",
            rbl_db::fixture::MY_TAG_PEAK,
            "10003",
            "--source",
            "dj",
        ],
    );
    assert_eq!(
        ok(&s, &["facts", "standing", "10003", "situation"])["standing"]["value"],
        serde_json::json!([])
    );
}

#[test]
fn a_fact_insert_failure_is_reported_without_losing_the_entry_or_blocking_repairs() {
    let s = Sandbox::new();
    ok(&s, &["tracks", "rate", "10003", "2", "--source", "dj"]);
    let db = rusqlite::Connection::open(s.sidecar_dir().join("sidecar.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_dj_fact BEFORE INSERT ON dj_fact BEGIN SELECT RAISE(ABORT, 'injected Fact failure'); END;").unwrap();
    let edit = ok(&s, &["tracks", "rate", "10003", "5", "--source", "dj"]);
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 5);
    for (table, expected) in [
        ("ledger", 2),
        ("fact", 1),
        ("dj_fact", 1),
        ("ledger_intent", 0),
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            expected
        );
    }
    let entry = ok(&s, &["runs", "show", &edit["run"]["id"].to_string()]);
    assert!(entry["entries"][0]["facts_not_recorded"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("injected Fact failure"));
    assert_eq!(
        ok(&s, &["facts", "standing", "10003", "energy"])["standing"]["value"],
        2
    );
    // The fault is still present: neither an agent nor a DJ repair is wedged.
    ok(
        &s,
        &["tracks", "rate", "10004", "4", "--source", "agent:repair"],
    );
    ok(&s, &["tracks", "rate", "10004", "3", "--source", "dj"]);
    db.execute_batch("DROP TRIGGER refuse_dj_fact").unwrap();
    ok(&s, &["tracks", "rate", "10003", "4", "--source", "dj"]);
    assert_eq!(
        ok(&s, &["facts", "standing", "10003", "energy"])["standing"]["value"],
        4
    );
}

#[test]
fn a_library_write_failure_clears_its_intent_without_a_fact_or_entry() {
    let s = Sandbox::new();
    s.edit(|writer| { writer.library().connection().execute_batch("CREATE TRIGGER refuse_rating BEFORE UPDATE OF Rating ON djmdContent BEGIN SELECT RAISE(ABORT, 'injected Library failure'); END;").unwrap(); });
    let failed = s.cli(&["--json", "tracks", "rate", "10003", "5", "--source", "dj"]);
    assert!(!failed.status.success());
    assert_eq!(ok(&s, &["tracks", "show", "10003"])["rating"], 0);
    assert_eq!(ok(&s, &["runs", "list"])["total"], 0);
    assert!(ok(&s, &["facts", "standing", "10003", "energy"])["standing"].is_null());
    let db = rusqlite::Connection::open(s.sidecar_dir().join("sidecar.db")).unwrap();
    for table in ["ledger", "fact", "ledger_intent", "dj_intent"] {
        assert_eq!(
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn a_hand_judgement_does_not_read_or_require_audio_identity() {
    let s = Sandbox::new();
    let track = s.import_track("Hand-edit.wav", 1);
    ok(&s, &["tracks", "rate", &track, "3", "--source", "dj"]);
    let fact = ok(&s, &["facts", "standing", &track, "energy"])["standing"].clone();
    assert_eq!(fact["value"], 3);
    assert!(fact["fingerprint"].is_null());
}

#[test]
fn a_single_text_schema_dimension_records_its_term_not_a_list() {
    let s = Sandbox::new();
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(
        s.sidecar_dir().join("tag-schema.toml"),
        r#"version = 1
[dimensions.vocals]
terms = [{id = "peak", name = "Peak"}, {id = "warm-up", name = "Warm-up"}]
[deck]
categories = [{dimension = "vocals", label = "Situation"}]
"#,
    )
    .unwrap();
    ok(
        &s,
        &[
            "mytags",
            "assign",
            rbl_db::fixture::MY_TAG_PEAK,
            "10003",
            "--source",
            "dj",
        ],
    );
    assert_eq!(
        ok(&s, &["facts", "standing", "10003", "vocals"])["standing"]["value"],
        "peak"
    );
    ok(
        &s,
        &[
            "mytags",
            "unassign",
            rbl_db::fixture::MY_TAG_PEAK,
            "10003",
            "--source",
            "dj",
        ],
    );
    assert!(ok(&s, &["facts", "standing", "10003", "vocals"])["standing"]["value"].is_null());
}
