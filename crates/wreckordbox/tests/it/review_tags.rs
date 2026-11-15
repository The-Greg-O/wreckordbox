//! A tag an agent proposed (wbx-45.26), through the real CLI and a service
//! on a fixture Library: it waits in Review beside Energy, the Intake queue
//! shows the very same row (its own token), and the DJ's accept is Review's
//! own decision. A deck dimension is applied as MyTags; a vocabulary
//! dimension is the DJ's Fact. Refused under Library Protection and on a
//! Stick-hosted Library, with nothing changed. A suggestion decided in
//! Review is gone from Intake. Bulk accept by confidence or `--all` still
//! leaves a proposed title, and does take a proposed tag.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;

use common::Sandbox;
use serde_json::Value;

use super::review::{fact, ok, refused};

const TRACK: &str = "10001";

const SCHEMA: &str = r#"version = 1

[dimensions.situation]
terms = [
  { id = "peak", name = "Peak" },
  { id = "warm-up", name = "Warm-up" },
]

[dimensions.texture]
terms = [{ id = "gritty", name = "Gritty" }]

[deck]
categories = [
  { dimension = "situation", label = "Situation" },
]
"#;

fn write_schema(s: &Sandbox) {
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join("tag-schema.toml"), SCHEMA).unwrap();
}

/// Records a list-of-texts Fact (a schema dimension's terms).
fn terms_from(
    s: &Sandbox,
    track: &str,
    kind: &str,
    values: &[&str],
    source: &str,
    confidence: &str,
) -> i64 {
    let mut args = vec!["facts", "record", track, kind];
    args.extend_from_slice(values);
    args.extend_from_slice(&["--source", source, "--confidence", confidence]);
    ok(s, &args)["facts"][0]["id"].as_i64().unwrap()
}

fn terms(s: &Sandbox, track: &str, kind: &str, values: &[&str], confidence: &str) -> i64 {
    terms_from(s, track, kind, values, "agent:tagger", confidence)
}

fn waiting(s: &Sandbox) -> Vec<Value> {
    ok(s, &["review", "waiting"])["waiting"]
        .as_array()
        .unwrap()
        .clone()
}

fn tag_rows(s: &Sandbox) -> Vec<Value> {
    waiting(s)
        .into_iter()
        .filter(|row| row["field"] == "tag")
        .collect()
}

fn intake_suggestions(s: &Sandbox) -> Vec<Value> {
    let page = ok(s, &["intake", "list"]);
    page["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == TRACK)
        .map(|t| t["suggestions"].as_array().unwrap().clone())
        .unwrap_or_default()
}

fn suggestion_for(s: &Sandbox, field: &str) -> Value {
    intake_suggestions(s)
        .into_iter()
        .find(|row| row["field"] == field)
        .unwrap_or_else(|| panic!("no {field} suggestion: {}", ok(s, &["intake", "list"])))
}

fn my_tags(s: &Sandbox) -> Vec<String> {
    let mut tags: Vec<String> = ok(s, &["mytags", "show", TRACK])["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| {
            format!(
                "{}: {}",
                tag["category"].as_str().unwrap(),
                tag["name"].as_str().unwrap()
            )
        })
        .collect();
    tags.sort();
    tags
}

fn accept(s: &Sandbox, facts: &[i64]) -> Value {
    let token = ok(s, &["review", "waiting"])["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut args = vec!["review".to_owned(), "accept".to_owned()];
    args.extend(facts.iter().map(ToString::to_string));
    args.extend([
        "--token".to_owned(),
        token,
        "--source".to_owned(),
        "dj".to_owned(),
    ]);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    ok(s, &borrowed)
}

fn proposed(s: &Sandbox) -> i64 {
    s.describe_track(1, "Old Title", "Old Artist", "8A", 0);
    write_schema(s);
    terms(s, TRACK, "situation", &["peak"], "0.84")
}

#[test]
fn a_proposed_tag_waits_in_review_and_intake_as_the_same_row() {
    let s = Sandbox::new();
    let id = proposed(&s);
    // The same terms the track already holds do not wait.
    ok(
        &s,
        &[
            "mytags",
            "assign",
            rbl_db::fixture::MY_TAG_PEAK,
            "10002",
            "--source",
            "dj",
        ],
    );
    terms(&s, "10002", "situation", &["peak"], "0.9");

    let rows = tag_rows(&s);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["fact"], id);
    assert_eq!(rows[0]["track"], TRACK);
    assert_eq!(rows[0]["field"], "tag");
    assert_eq!(rows[0]["reason"], "waits_for_dj");
    assert_eq!(rows[0]["after"]["dimension"], "situation");
    assert_eq!(rows[0]["after"]["terms"], serde_json::json!(["peak"]));
    assert_eq!(rows[0]["after"]["label"], "Peak");
    assert_eq!(rows[0]["after"]["dimension_label"], "Situation");
    assert_eq!(rows[0]["before"]["terms"], serde_json::json!([]));
    assert!(rows[0]["threshold"].is_null());

    let all = ok(&s, &["review", "waiting"]);
    let suggestion = suggestion_for(&s, "situation");
    assert_eq!(suggestion["fact_id"], id);
    assert_eq!(suggestion["value"], "Peak");
    assert_eq!(suggestion["terms"], serde_json::json!(["peak"]));
    assert_eq!(
        suggestion["token"],
        all["row_tokens"][id.to_string()],
        "{suggestion}"
    );
    // Track 10002 already holds Peak, so its proposal is not offered.
    let page = ok(&s, &["intake", "list"]);
    let other = page["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "10002")
        .unwrap();
    assert!(
        other["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["field"] != "situation"),
        "{other}"
    );
}

#[test]
fn accepting_a_proposed_tag_is_the_djs_decision_and_leaves_review_and_intake() {
    let s = Sandbox::new();
    let situation = proposed(&s);
    let texture = terms(&s, TRACK, "texture", &["gritty"], "0.7");

    let decided = accept(&s, &[situation, texture]);
    let taken: Vec<i64> = decided["decided"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["fact"].as_i64().unwrap())
        .collect();
    assert_eq!(taken.len(), 2, "{decided}");
    assert!(
        taken.contains(&situation) && taken.contains(&texture),
        "{decided}"
    );

    assert_eq!(my_tags(&s), vec!["Situation: Peak".to_owned()]);
    let standing = ok(&s, &["facts", "standing", TRACK, "texture"])["standing"].clone();
    assert_eq!(standing["source"]["kind"], "dj", "{standing}");
    assert_eq!(
        standing["value"],
        serde_json::json!(["gritty"]),
        "{standing}"
    );
    // The deck dimension's accept is the DJ's Fact of that kind too.
    let situation_fact = ok(&s, &["facts", "standing", TRACK, "situation"])["standing"].clone();
    assert_eq!(situation_fact["source"]["kind"], "dj", "{situation_fact}");

    assert!(tag_rows(&s).is_empty(), "{:?}", tag_rows(&s));
    assert!(
        intake_suggestions(&s)
            .iter()
            .all(|row| row["field"] != "situation" && row["field"] != "texture"),
        "{:?}",
        intake_suggestions(&s)
    );
}

#[test]
fn a_tag_rejected_in_review_is_gone_from_intake_and_stays_rejected() {
    let s = Sandbox::new();
    let situation = proposed(&s);
    let texture = terms(&s, TRACK, "texture", &["gritty"], "0.7");

    ok(
        &s,
        &["review", "reject", &situation.to_string(), "--source", "dj"],
    );

    let left: Vec<String> = intake_suggestions(&s)
        .iter()
        .map(|row| row["field"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(left, ["texture".to_owned()]);
    assert!(my_tags(&s).is_empty());
    // The same terms from the same source stay rejected.
    terms(&s, TRACK, "situation", &["peak"], "0.95");
    assert!(
        tag_rows(&s)
            .iter()
            .all(|row| row["fact"] != situation && row["after"]["dimension"] != "situation"),
        "{:?}",
        tag_rows(&s)
    );
    assert!(tag_rows(&s).iter().any(|row| row["fact"] == texture));
    // Another answer is a new suggestion.
    let other = terms(&s, TRACK, "situation", &["warm-up"], "0.95");
    assert!(tag_rows(&s).iter().any(|row| row["fact"] == other));
}

#[test]
fn an_accept_under_library_protection_changes_nothing_and_the_tag_still_waits() {
    let s = Sandbox::new();
    let id = proposed(&s);
    s.protect(true);

    let error = {
        let token = ok(&s, &["review", "waiting"])["token"]
            .as_str()
            .unwrap()
            .to_owned();
        refused(
            &s,
            &[
                "review",
                "accept",
                &id.to_string(),
                "--token",
                &token,
                "--source",
                "dj",
            ],
        )
    };
    assert_eq!(error["code"], "library_protected", "{error}");
    s.protect(false);
    assert!(my_tags(&s).is_empty());
    assert_eq!(tag_rows(&s)[0]["fact"], id);
    assert_eq!(suggestion_for(&s, "situation")["fact_id"], id);
}

#[test]
fn an_accept_on_a_stick_hosted_library_changes_nothing_and_the_tag_still_waits() {
    let s = Sandbox::new();
    let id = proposed(&s);
    let sidecar = s.sidecar_dir();
    std::fs::write(
        sidecar.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let name = sidecar.file_name().unwrap().to_str().unwrap().to_owned();
    let unlocked = s.cli(&[
        "stick",
        "unlock",
        sidecar.to_str().unwrap(),
        "--name",
        &name,
    ]);
    assert!(unlocked.status.success(), "{}", unlocked.stderr);

    let token = ok(&s, &["review", "waiting"])["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let error = refused(
        &s,
        &[
            "review",
            "accept",
            &id.to_string(),
            "--token",
            &token,
            "--source",
            "dj",
        ],
    );
    assert_eq!(error["code"], "sidecar_on_stick", "{error}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Nothing was written"),
        "{error}"
    );
    assert!(my_tags(&s).is_empty());
    assert_eq!(tag_rows(&s)[0]["fact"], id);
    assert_eq!(suggestion_for(&s, "situation")["fact_id"], id);
}

#[test]
fn a_tag_changed_since_it_was_shown_refuses_the_accept() {
    let s = Sandbox::new();
    let id = proposed(&s);
    let token = ok(&s, &["review", "waiting"])["token"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(
        &s,
        &[
            "mytags",
            "assign",
            rbl_db::fixture::MY_TAG_PEAK,
            TRACK,
            "--source",
            "dj",
        ],
    );

    let error = refused(
        &s,
        &[
            "review",
            "accept",
            &id.to_string(),
            "--token",
            &token,
            "--source",
            "dj",
        ],
    );
    assert_eq!(error["code"], "review_changed", "{error}");
    assert_eq!(my_tags(&s), vec!["Situation: Peak".to_owned()]);
}

#[test]
fn a_bulk_accept_takes_a_proposed_tag_and_never_a_proposed_title() {
    let s = Sandbox::new();
    let situation = proposed(&s);
    let title = fact(&s, TRACK, "title", "A Better Title", "agent:tagger", "0.9");
    let energy = fact(&s, TRACK, "energy", "4", "agent:tagger", "0.95");
    let token = ok(&s, &["review", "waiting"])["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let decided = ok(
        &s,
        &[
            "review", "accept", "--all", "--token", &token, "--source", "dj",
        ],
    );
    let taken: Vec<i64> = decided["decided"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["fact"].as_i64().unwrap())
        .collect();
    assert!(taken.contains(&situation), "{decided}");
    assert!(taken.contains(&energy), "{decided}");
    assert!(!taken.contains(&title), "{decided}");
    assert_eq!(my_tags(&s), vec!["Situation: Peak".to_owned()]);
    assert_eq!(ok(&s, &["tracks", "show", TRACK])["title"], "Old Title");
    let left: Vec<i64> = intake_suggestions(&s)
        .iter()
        .map(|row| row["fact_id"].as_i64().unwrap())
        .collect();
    assert_eq!(left, vec![title]);
}

#[test]
fn two_proposals_for_one_dimension_compete_and_two_dimensions_do_not() {
    let s = Sandbox::new();
    let peak = proposed(&s);
    // A second source, so this stays live beside the first proposal.
    let warm = terms_from(&s, TRACK, "situation", &["warm-up"], "agent:other", "0.5");
    let texture = terms(&s, TRACK, "texture", &["gritty"], "0.7");
    let token = ok(&s, &["review", "waiting"])["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let error = refused(
        &s,
        &[
            "review",
            "accept",
            &peak.to_string(),
            &warm.to_string(),
            "--token",
            &token,
            "--source",
            "dj",
        ],
    );
    assert_eq!(error["code"], "review_competing", "{error}");
    assert!(my_tags(&s).is_empty());

    // The vocabulary dimension is a different proposal and can be taken
    // with one of the situation rows.
    let token = ok(&s, &["review", "waiting"])["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let decided = ok(
        &s,
        &[
            "review",
            "accept",
            &peak.to_string(),
            &texture.to_string(),
            "--token",
            &token,
            "--source",
            "dj",
        ],
    );
    assert_eq!(decided["count"], 2, "{decided}");
    assert_eq!(my_tags(&s), vec!["Situation: Peak".to_owned()]);
    // The DJ's Situation Fact is newer than the warm-up proposal, so that
    // older proposal ends with the accept. A newer one waits again.
    assert!(
        tag_rows(&s).iter().all(|row| row["fact"] != warm),
        "{:?}",
        tag_rows(&s)
    );
    let again = terms_from(&s, TRACK, "situation", &["warm-up"], "agent:other", "0.5");
    assert!(tag_rows(&s).iter().any(|row| row["fact"] == again));
}
