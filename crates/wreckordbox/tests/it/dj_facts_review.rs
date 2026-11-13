//! Regression probes from the independent native-edit review.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;
use common::Sandbox;
use rbl_db::fixture::{MY_TAG_PEAK, MY_TAG_WARM_UP};
use serde_json::{json, Value};
fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let out = s.cli(&all);
    assert!(out.status.success(), "{args:?}: {}", out.stderr);
    out.json()
}
fn standing(s: &Sandbox, kind: &str) -> Value {
    ok(s, &["facts", "standing", "10003", kind])["standing"].clone()
}
fn entries(s: &Sandbox, edit: &Value) -> Value {
    ok(s, &["runs", "show", &edit["run"]["id"].to_string()])["entries"].clone()
}
fn schema(s: &Sandbox, text: &str) {
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join("tag-schema.toml"), text).unwrap();
}
#[test]
fn broken_schema_does_not_refuse_a_rating_or_hide_unmapped_tag_work() {
    let s = Sandbox::new();
    schema(&s, "not valid [");
    ok(&s, &["tracks", "rate", "10003", "3", "--source", "dj"]);
    assert_eq!(standing(&s, "energy")["value"], 3);
    let edit = ok(
        &s,
        &["mytags", "assign", MY_TAG_PEAK, "10003", "--source", "dj"],
    );
    assert!(!entries(&s, &edit)[0]["facts_not_recorded"]
        .as_array()
        .unwrap()
        .is_empty());
    ok(
        &s,
        &[
            "mytags",
            "unassign",
            MY_TAG_PEAK,
            "10003",
            "--source",
            "agent:repair",
        ],
    );
}
#[test]
fn impossible_dimension_is_reported_and_never_blocks_its_repair() {
    let s = Sandbox::new();
    schema(
        &s,
        r#"version=1
[dimensions.vocals]
terms=[{id="peak",name="Peak"},{id="warm-up",name="Warm-up"}]
[deck]
categories=[{dimension="vocals",label="Situation"}]
"#,
    );
    ok(
        &s,
        &["mytags", "assign", MY_TAG_PEAK, "10003", "--source", "dj"],
    );
    let edit = ok(
        &s,
        &[
            "mytags",
            "assign",
            MY_TAG_WARM_UP,
            "10003",
            "--source",
            "dj",
        ],
    );
    assert_eq!(entries(&s, &edit).as_array().unwrap().len(), 1);
    assert!(!entries(&s, &edit)[0]["facts_not_recorded"]
        .as_array()
        .unwrap()
        .is_empty());
    ok(
        &s,
        &[
            "mytags",
            "unassign",
            MY_TAG_WARM_UP,
            "10003",
            "--source",
            "dj",
        ],
    );
    assert_eq!(standing(&s, "vocals")["value"], "peak");
    ok(
        &s,
        &["tracks", "rate", "10004", "4", "--source", "agent:repair"],
    );
}
#[test]
fn vocabulary_rename_preserves_the_judgement_and_term_identity() {
    let s = Sandbox::new();
    ok(&s, &["schema", "convert"]);
    ok(
        &s,
        &["mytags", "assign", MY_TAG_PEAK, "10003", "--source", "dj"],
    );
    let before = standing(&s, "situation");
    ok(
        &s,
        &[
            "mytags",
            "rename",
            MY_TAG_PEAK,
            "Peak time",
            "--source",
            "dj",
        ],
    );
    assert_eq!(standing(&s, "situation"), before);
}
#[test]
fn missing_or_unmapped_schema_has_an_entry_with_a_visible_reason() {
    for mapped in [false, true] {
        let s = Sandbox::new();
        if mapped {
            schema(
                &s,
                r#"version=1
[dimensions.situation]
terms=[{id="different",name="Different"}]
[deck]
categories=[{dimension="situation",label="Situation"}]
"#,
            );
        }
        let edit = ok(
            &s,
            &["mytags", "assign", MY_TAG_PEAK, "10003", "--source", "dj"],
        );
        let entries = entries(&s, &edit);
        assert!(
            !entries[0]["facts_not_recorded"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{entries}"
        );
        ok(&s, &["tracks", "rate", "10003", "2", "--source", "dj"]);
    }
}
#[test]
fn term_order_matches_conversion_instead_of_alphabetical_order() {
    let s = Sandbox::new();
    let after = s.create_my_tag_in(rbl_db::fixture::MY_TAG_CATEGORY, "Afterhours");
    ok(&s, &["schema", "convert"]);
    ok(
        &s,
        &["mytags", "assign", MY_TAG_PEAK, "10003", "--source", "dj"],
    );
    ok(&s, &["mytags", "assign", &after, "10003", "--source", "dj"]);
    assert_eq!(
        standing(&s, "situation")["value"],
        json!(["peak", "afterhours"])
    );
    let converted = ok(&s, &["schema", "convert"]);
    assert_eq!(converted["facts_conflicting"], 0, "{converted}");
}
#[test]
fn damaged_audio_cannot_prevent_a_judgement_or_a_later_edit() {
    let s = Sandbox::new();
    let track = s.import_track("Broken.wav", 1);
    s.edit(|w| {
        let row = w.library().stored_path(&track).unwrap().unwrap();
        let path = w.library().track_paths().resolve(&row);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
    });
    let edit = ok(&s, &["tracks", "rate", &track, "3", "--source", "dj"]);
    assert_eq!(entries(&s, &edit).as_array().unwrap().len(), 1);
    ok(
        &s,
        &["tracks", "rate", &track, "4", "--source", "agent:repair"],
    );
}

#[test]
fn default_and_joined_agent_sources_never_create_dj_facts() {
    let s = Sandbox::new();
    ok(&s, &["tracks", "rate", "10003", "4"]);
    assert!(standing(&s, "energy").is_null());
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:projection",
            "--description",
            "Project tags",
        ],
    );
    ok(
        &s,
        &[
            "tracks",
            "rate",
            "10003",
            "5",
            "--run",
            &run["id"].to_string(),
        ],
    );
    assert!(standing(&s, "energy").is_null());
    let shown = ok(&s, &["runs", "show", &run["id"].to_string()]);
    assert_eq!(shown["entries"][0]["source"]["kind"], "agent");
}
#[test]
fn mixed_native_membership_does_not_promote_unchanged_agent_terms() {
    let s = Sandbox::new();
    ok(&s, &["schema", "convert"]);
    ok(
        &s,
        &[
            "mytags",
            "assign",
            MY_TAG_WARM_UP,
            "10003",
            "--source",
            "agent:projection",
        ],
    );
    let edit = ok(
        &s,
        &["mytags", "assign", MY_TAG_PEAK, "10003", "--source", "dj"],
    );
    assert!(standing(&s, "situation").is_null());
    let entry = entries(&s, &edit);
    assert!(entry[0]["facts_not_recorded"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("provenance"));
}
#[test]
fn broken_kinds_configuration_does_not_refuse_native_edits() {
    let s = Sandbox::new();
    ok(&s, &["schema", "convert"]);
    std::fs::write(s.sidecar_dir().join("kinds.toml"), "invalid [").unwrap();
    ok(&s, &["tracks", "rate", "10003", "3", "--source", "dj"]);
    let edit = ok(
        &s,
        &["mytags", "assign", MY_TAG_PEAK, "10003", "--source", "dj"],
    );
    assert!(!entries(&s, &edit)[0]["facts_not_recorded"]
        .as_array()
        .unwrap()
        .is_empty());
    ok(
        &s,
        &[
            "mytags",
            "unassign",
            MY_TAG_PEAK,
            "10003",
            "--source",
            "agent:repair",
        ],
    );
    std::fs::remove_file(s.sidecar_dir().join("kinds.toml")).unwrap();
    assert_eq!(standing(&s, "energy")["value"], 3);
}
#[test]
fn run_text_explains_why_a_native_fact_was_skipped() {
    let s = Sandbox::new();
    let edit = ok(
        &s,
        &["mytags", "assign", MY_TAG_PEAK, "10003", "--source", "dj"],
    );
    let text = s.cli(&["runs", "show", &edit["run"]["id"].to_string()]);
    assert!(text.status.success());
    assert!(text.stdout.contains("Fact not recorded"), "{}", text.stdout);
    assert!(text.stdout.contains("Tag schema unavailable"));
}
