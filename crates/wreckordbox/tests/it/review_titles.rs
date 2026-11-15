//! A title or an artist an agent proposed (wbx-66), through the real CLI and
//! a service on a fixture Library: it waits in Review beside Energy and Prep
//! colour, the Intake queue shows the very same row (its own token), and the
//! DJ's accept is the one Review decision: declared, bound to what was
//! shown, a hand set through the edit path recorded as the DJ's, refused
//! under Library Protection with nothing changed. A suggestion decided in
//! Review is gone from Intake.
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

fn proposed(s: &Sandbox) -> (i64, i64) {
    s.describe_track(1, "Old Title", "Old Artist", "8A", 0);
    let title = fact(s, TRACK, "title", "A Better Title", "agent:tagger", "0.7");
    let artist = fact(s, TRACK, "artist", "Someone Else", "agent:tagger", "0.6");
    (title, artist)
}

fn waiting_rows(s: &Sandbox, field: &str) -> Value {
    ok(s, &["review", "waiting", "--field", field])
}

/// The Intake queue's suggestions for `TRACK`.
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

fn show(s: &Sandbox) -> Value {
    ok(s, &["tracks", "show", TRACK])
}

#[test]
fn a_proposed_title_and_artist_wait_in_review_and_intake_as_the_same_rows() {
    let s = Sandbox::new();
    let (title, artist) = proposed(&s);

    let titles = waiting_rows(&s, "title");
    let rows = titles["waiting"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{titles}");
    assert_eq!(rows[0]["fact"], title);
    assert_eq!(rows[0]["track"], TRACK);
    assert_eq!(rows[0]["field"], "title");
    assert_eq!(rows[0]["before"], "Old Title");
    assert_eq!(rows[0]["after"], "A Better Title");
    assert_eq!(rows[0]["source"]["name"], "tagger");
    let artists = waiting_rows(&s, "artist");
    assert_eq!(artists["waiting"][0]["fact"], artist);
    assert_eq!(artists["waiting"][0]["before"], "Old Artist");
    assert_eq!(artists["waiting"][0]["after"], "Someone Else");

    // Intake shows the same rows with the same tokens Review gives them.
    let all = ok(&s, &["review", "waiting"]);
    let suggestions = intake_suggestions(&s);
    assert_eq!(suggestions.len(), 2, "{suggestions:?}");
    for suggestion in &suggestions {
        let id = suggestion["fact_id"].as_i64().unwrap();
        assert_eq!(
            suggestion["token"],
            all["row_tokens"][id.to_string()],
            "{suggestion}"
        );
    }
}

#[test]
fn accepting_a_proposed_title_is_the_djs_hand_set_and_leaves_review_and_intake() {
    let s = Sandbox::new();
    let (title, artist) = proposed(&s);
    let token = waiting_rows(&s, "title")["token"]
        .as_str()
        .unwrap()
        .to_owned();

    let decided = ok(
        &s,
        &[
            "review",
            "accept",
            &title.to_string(),
            "--token",
            &token,
            "--source",
            "dj",
        ],
    );
    assert_eq!(decided["count"], 1, "{decided}");
    assert_eq!(show(&s)["title"], "A Better Title");
    assert_eq!(show(&s)["artist"], "Old Artist");

    // Recorded as the DJ's own: a DJ title Fact with the accepted value.
    let standing = ok(&s, &["facts", "standing", TRACK, "title"])["standing"].clone();
    assert_eq!(standing["source"]["kind"], "dj", "{standing}");
    assert_eq!(standing["value"], "A Better Title", "{standing}");

    // Gone from Review and from Intake; the artist still waits in both.
    assert!(waiting_rows(&s, "title")["waiting"]
        .as_array()
        .unwrap()
        .is_empty());
    let left: Vec<i64> = intake_suggestions(&s)
        .iter()
        .map(|s| s["fact_id"].as_i64().unwrap())
        .collect();
    assert_eq!(left, [artist]);

    // The artist, by its own listing.
    let token = waiting_rows(&s, "artist")["token"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(
        &s,
        &[
            "review",
            "accept",
            &artist.to_string(),
            "--token",
            &token,
            "--source",
            "dj",
        ],
    );
    assert_eq!(show(&s)["artist"], "Someone Else");
    assert!(intake_suggestions(&s).is_empty());
}

#[test]
fn a_proposed_title_rejected_in_review_is_gone_from_intake_and_stays_rejected() {
    let s = Sandbox::new();
    let (title, artist) = proposed(&s);

    ok(
        &s,
        &["review", "reject", &title.to_string(), "--source", "dj"],
    );

    let left: Vec<i64> = intake_suggestions(&s)
        .iter()
        .map(|s| s["fact_id"].as_i64().unwrap())
        .collect();
    assert_eq!(left, [artist]);
    assert_eq!(show(&s)["title"], "Old Title");
    // The same answer recorded again is the same suggestion: still rejected.
    fact(&s, TRACK, "title", "A Better Title", "agent:tagger", "0.9");
    assert!(waiting_rows(&s, "title")["waiting"]
        .as_array()
        .unwrap()
        .is_empty());
    // Another answer is a new suggestion.
    let other = fact(&s, TRACK, "title", "Another Title", "agent:tagger", "0.9");
    assert_eq!(waiting_rows(&s, "title")["waiting"][0]["fact"], other);
}

#[test]
fn an_accept_under_library_protection_changes_nothing_and_the_title_still_waits() {
    let s = Sandbox::new();
    let (title, _) = proposed(&s);
    let token = waiting_rows(&s, "title")["token"]
        .as_str()
        .unwrap()
        .to_owned();
    s.protect(true);

    let error = refused(
        &s,
        &[
            "review",
            "accept",
            &title.to_string(),
            "--token",
            &token,
            "--source",
            "dj",
        ],
    );
    assert_eq!(error["code"], "library_protected", "{error}");
    s.protect(false);
    assert_eq!(show(&s)["title"], "Old Title");
    assert_eq!(waiting_rows(&s, "title")["waiting"][0]["fact"], title);
    assert_eq!(intake_suggestions(&s).len(), 2);
}

#[test]
fn a_title_written_since_it_was_shown_refuses_the_accept_and_is_kept() {
    let s = Sandbox::new();
    let (title, _) = proposed(&s);
    let token = waiting_rows(&s, "title")["token"]
        .as_str()
        .unwrap()
        .to_owned();
    // Written by another program after the DJ was shown the row.
    s.edit(|w| {
        w.set_field(TRACK, rbl_db::write::TrackField::Title, "Typed Elsewhere")
            .unwrap();
    });

    let error = refused(
        &s,
        &[
            "review",
            "accept",
            &title.to_string(),
            "--token",
            &token,
            "--source",
            "dj",
        ],
    );
    assert_eq!(error["code"], "review_changed", "{error}");
    let stored = rbl_db::Library::open(s.location().clone(), rbl_db::OpenMode::ReadOnly)
        .unwrap()
        .connection()
        .query_row(
            "SELECT Title FROM djmdContent WHERE ID = ?1",
            [TRACK],
            |r| r.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(stored, "Typed Elsewhere");
}

#[test]
fn a_bulk_accept_by_filter_or_all_never_takes_a_proposed_title_or_artist() {
    let s = Sandbox::new();
    let (title, _) = proposed(&s);
    // An Energy Fact waits beside them (the policy waits by default).
    let energy = fact(&s, TRACK, "energy", "4", "agent:tagger", "0.95");
    let token = ok(&s, &["review", "waiting"])["token"]
        .as_str()
        .unwrap()
        .to_owned();

    // Everything, and everything at or above a confidence: Energy only.
    let decided = ok(
        &s,
        &[
            "review",
            "accept",
            "--at-least",
            "0.5",
            "--token",
            &token,
            "--source",
            "dj",
        ],
    );
    let decided_facts: Vec<i64> = decided["decided"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["fact"].as_i64().unwrap())
        .collect();
    assert_eq!(decided_facts, [energy], "{decided}");
    assert_eq!(show(&s)["title"], "Old Title");
    assert_eq!(show(&s)["artist"], "Old Artist");
    assert_eq!(intake_suggestions(&s).len(), 2);
    let token = ok(&s, &["review", "waiting"])["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let all = ok(
        &s,
        &[
            "review", "accept", "--all", "--token", &token, "--source", "dj",
        ],
    );
    assert_eq!(all["count"], 0, "{all}");
    assert_eq!(show(&s)["title"], "Old Title");

    // A filter naming the field is refused: a proposal is accepted by name.
    let token = waiting_rows(&s, "title")["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let error = refused(
        &s,
        &[
            "review", "accept", "--field", "title", "--token", &token, "--source", "dj",
        ],
    );
    assert_eq!(error["code"], "bad_request", "{error}");
    assert_eq!(show(&s)["title"], "Old Title");
    assert_eq!(waiting_rows(&s, "title")["waiting"][0]["fact"], title);
}
