//! What the Review view (wbx-74) needs from the service beyond wbx-27,
//! through the real CLI and a service on a fixture Library: each waiting
//! Fact's Run, so the view can group by Run (still a read: nothing
//! written, no event), and an event when the DJ's decision changes the
//! queue without touching the Library (a rejection or a reopening), so a
//! window showing the queue hears another client's decision.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;

use common::Sandbox;
use serde_json::{json, Value};

use super::review::{events_before_barriers, fact, ok};

fn sidecar_bytes(s: &Sandbox) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(s.sidecar_dir()) {
        for entry in entries.flatten() {
            if entry.path().is_file() {
                out.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

/// Drains what was sent before (an ordered barrier, not a quiet window).
fn quiet(s: &Sandbox, watcher: &mut wreckordbox_service::Client) {
    events_before_barriers(s, watcher);
}

#[test]
fn each_waiting_fact_names_its_run_and_the_listing_lists_those_runs_writing_nothing() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let run = ok(
        &s,
        &[
            "runs",
            "start",
            "--source",
            "agent:tagger",
            "--description",
            "tagging pass",
        ],
    );
    let run_id = run["id"].as_i64().unwrap();
    let in_run = |track: &str, value: &str, confidence: &str| {
        ok(
            &s,
            &[
                "facts",
                "record",
                track,
                "energy",
                value,
                "--run",
                &run_id.to_string(),
                "--confidence",
                confidence,
            ],
        )["facts"][0]["id"]
            .as_i64()
            .unwrap()
    };
    let a = in_run("10001", "4", "0.9");
    let b = in_run("10002", "2", "0.5");
    // Recorded on its own: a Run of its own, finished with the command.
    let alone = fact(&s, "10003", "prep_colour", "Green", "tool:meter", "0.8");
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    quiet(&s, &mut watcher);
    let bytes_before = sidecar_bytes(&s);
    let library_before = std::fs::read(s.master_db()).unwrap();

    let listing = ok(&s, &["review", "waiting"]);

    let runs = listing["runs"]
        .as_array()
        .expect("the listing names its Runs");
    assert_eq!(runs.len(), 2, "{runs:?}");
    let tagging = runs.iter().find(|r| r["id"] == run_id).unwrap();
    assert_eq!(
        tagging["source"],
        json!({"kind": "agent", "name": "tagger"})
    );
    assert_eq!(tagging["description"], "tagging pass");
    assert!(tagging["started_at"].as_i64().unwrap() > 0);
    // Still working: nothing finished it.
    assert_eq!(tagging["finished_at"], Value::Null);
    assert_eq!(tagging["facts"], json!([a, b]));
    let other = runs.iter().find(|r| r["id"] != run_id).unwrap();
    assert_eq!(other["source"], json!({"kind": "tool", "name": "meter"}));
    assert!(other["finished_at"].as_i64().is_some(), "{other:?}");
    assert_eq!(other["facts"], json!([alone]));
    // Every waiting Fact is in exactly one listed Run.
    let mut grouped: Vec<i64> = runs
        .iter()
        .flat_map(|r| {
            r["facts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f.as_i64().unwrap())
        })
        .collect();
    grouped.sort_unstable();
    let mut listed: Vec<i64> = listing["waiting"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["fact"].as_i64().unwrap())
        .collect();
    listed.sort_unstable();
    assert_eq!(grouped, listed);

    // A read: no Sidecar byte, no Library byte, no event.
    assert_eq!(sidecar_bytes(&s), bytes_before);
    assert_eq!(std::fs::read(s.master_db()).unwrap(), library_before);
    let stray = events_before_barriers(&s, &mut watcher);
    assert!(stray.is_empty(), "{stray:?}");

    // Finishing the Run is seen by the next listing.
    ok(&s, &["runs", "finish", &run_id.to_string()]);
    let again = ok(&s, &["review", "waiting"]);
    let tagging = again["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == run_id)
        .unwrap()
        .clone();
    assert!(tagging["finished_at"].as_i64().is_some(), "{tagging:?}");
    // The token covers what an accept would do, not the Runs: unchanged.
    assert_eq!(again["token"], listing["token"]);
}

#[test]
fn a_rejection_and_a_reopening_are_told_to_every_client_and_nothing_else_is() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let energy = fact(&s, "10001", "energy", "4", "agent:tagger", "0.7");
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    quiet(&s, &mut watcher);

    let rejected = ok(
        &s,
        &["review", "reject", &energy.to_string(), "--source", "dj"],
    );
    let events = events_before_barriers(&s, &mut watcher);
    let decided: Vec<&Value> = events
        .iter()
        .filter(|(name, _)| name == "review_decided")
        .map(|(_, data)| data)
        .collect();
    assert_eq!(decided.len(), 1, "{events:?}");
    assert_eq!(decided[0]["decision"], "rejected");
    assert_eq!(decided[0]["facts"], json!([energy]));
    assert!(
        events.iter().all(|(name, _)| name != "library_changed"),
        "{events:?}"
    );

    let entry = rejected["rejection"]["entry"].as_i64().unwrap();
    ok(
        &s,
        &[
            "review",
            "reopen",
            "--rejection",
            &entry.to_string(),
            "--source",
            "dj",
        ],
    );
    let events = events_before_barriers(&s, &mut watcher);
    let decided: Vec<&Value> = events
        .iter()
        .filter(|(name, _)| name == "review_decided")
        .map(|(_, data)| data)
        .collect();
    assert_eq!(decided.len(), 1, "{events:?}");
    assert_eq!(decided[0]["decision"], "reopened");
    assert_eq!(decided[0]["facts"], json!([energy]));

    // A refused decision tells nobody.
    let refused = s.cli(&["--json", "review", "reject", &energy.to_string()]);
    assert!(!refused.status.success());
    let stray = events_before_barriers(&s, &mut watcher);
    assert!(stray.is_empty(), "{stray:?}");
}

/// The listing's per-row tokens (wbx-74 review): `row_tokens[fact]`.
fn row_token(listing: &Value, fact: i64) -> String {
    listing["row_tokens"][fact.to_string()]
        .as_str()
        .unwrap_or_else(|| panic!("no row token for {fact}: {listing}"))
        .to_owned()
}

fn accept_rows(s: &Sandbox, rows: Value) -> Result<Value, wreckordbox_service::ServiceError> {
    s.client().call::<Value>(
        "review.accept",
        json!({ "rows": rows, "source": {"kind": "dj"} }),
    )
}

#[test]
fn an_accept_of_rows_by_their_own_tokens_is_not_refused_by_an_unrelated_arrival() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let shown = ok(&s, &["review", "waiting"]);
    let token = row_token(&shown, a);
    // Another Run records a Fact for another track: the whole listing's
    // token is stale now, the row's is not.
    fact(&s, "10002", "energy", "2", "agent:other", "0.7");
    let whole = s.cli(&[
        "--json",
        "review",
        "accept",
        &a.to_string(),
        "--token",
        shown["token"].as_str().unwrap(),
        "--source",
        "dj",
    ]);
    assert!(
        !whole.status.success(),
        "the whole-list token still binds the CLI"
    );

    let accepted = accept_rows(&s, json!([{ "fact": a, "token": token }])).unwrap();

    assert_eq!(accepted["count"], 1, "{accepted}");
    assert_eq!(super::review::rating(&s, "10001"), 4);
}

#[test]
fn a_stale_row_is_refused_by_name_and_no_selected_row_is_applied() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let b = fact(&s, "10002", "energy", "2", "agent:tagger", "0.9");
    let shown = ok(&s, &["review", "waiting"]);
    let rows = json!([
        { "fact": a, "token": row_token(&shown, a) },
        { "fact": b, "token": row_token(&shown, b) },
    ]);
    // The same source changes its mind about track 10002: Fact b is
    // superseded, no longer waiting.
    fact(&s, "10002", "energy", "5", "agent:tagger", "0.9");

    let refused = accept_rows(&s, rows).unwrap_err();

    assert_eq!(refused.code, "review_changed", "{refused:?}");
    assert!(
        refused.message.contains(&format!("Fact {b}")),
        "{}",
        refused.message
    );
    assert!(
        !refused.message.contains(&format!("Fact {a} ")),
        "{}",
        refused.message
    );
    // All or nothing: a, whose row is unchanged, was not applied either.
    assert_eq!(super::review::rating(&s, "10001"), 0);
    let waiting = super::review::facts_waiting(&s);
    assert!(waiting.contains(&a), "{waiting:?}");
}

#[test]
fn rows_are_for_an_accept_only_and_never_with_another_selection() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let shown = ok(&s, &["review", "waiting"]);
    let row = json!([{ "fact": a, "token": row_token(&shown, a) }]);
    let mut client = s.client();
    for (command, args) in [
        (
            "review.reject",
            json!({ "rows": row, "source": {"kind": "dj"} }),
        ),
        (
            "review.reopen",
            json!({ "rows": row, "source": {"kind": "dj"} }),
        ),
        ("review.waiting", json!({ "rows": row })),
        (
            "review.accept",
            json!({ "rows": row, "token": shown["token"], "source": {"kind": "dj"} }),
        ),
        (
            "review.accept",
            json!({ "rows": row, "facts": [a], "source": {"kind": "dj"} }),
        ),
    ] {
        let refused = client.call::<Value>(command, args).unwrap_err();
        assert_eq!(refused.code, "bad_request", "{command}: {refused:?}");
    }
    // Without the DJ declared: refused as before.
    let refused = client
        .call::<Value>("review.accept", json!({ "rows": row }))
        .unwrap_err();
    assert_eq!(refused.code, "review_not_the_djs");
    assert_eq!(super::review::facts_waiting(&s), vec![a]);
}

fn accept_refused(s: &Sandbox, rows: Value) -> wreckordbox_service::ServiceError {
    let refused = accept_rows(s, rows).unwrap_err();
    assert_eq!(refused.code, "review_changed", "{refused:?}");
    refused
}

#[test]
fn a_real_row_token_is_refused_once_its_fact_is_reattached_to_another_track_and_nothing_is_written()
{
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    super::review_races::describe(&s, &id);
    ok(&s, &["status"]);
    let f = fact(&s, &id, "energy", "4", "agent:tagger", "0.7");
    let shown = ok(&s, &["review", "waiting"]);
    let row = json!([{ "fact": f, "token": row_token(&shown, f) }]);
    let (next, _service) = super::review_races::readd(&s, &id);

    let refused = accept_refused(&s, row);

    assert!(
        refused
            .message
            .contains(&format!("Fact {f} is now: track {next}")),
        "{}",
        refused.message
    );
    assert_eq!(super::review_races::db_rating(&s, &next), 0);
    assert_eq!(super::review::facts_waiting(&s), vec![f]);
}

#[test]
fn a_real_row_token_is_refused_when_why_the_fact_waits_changes() {
    // Energy and Prep colour are Judgements: they never go stale, and a
    // Fact's proposed value never changes under its id (a newer Fact
    // supersedes it, below). What can change under the same id is the
    // track (above) and its eligibility: the policy now holds it for
    // another reason.
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let f = fact(&s, "10001", "energy", "4", "agent:tagger", "0.6");
    let shown = ok(&s, &["review", "waiting"]);
    assert_eq!(shown["waiting"][0]["reason"], "waits_for_dj");
    let row = json!([{ "fact": f, "token": row_token(&shown, f) }]);
    super::review::policy(&s, "[energy]\napply = 0.8\n");
    assert_eq!(super::review::waiting(&s)[0]["reason"], "below_confidence");

    let refused = accept_refused(&s, row);

    assert!(
        refused.message.contains(&format!("Fact {f} is now")),
        "{}",
        refused.message
    );
    assert_eq!(super::review::rating(&s, "10001"), 0);
}

#[test]
fn every_changed_row_is_named_and_no_row_is_applied() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let b = fact(&s, "10002", "energy", "2", "agent:tagger", "0.9");
    let c = fact(&s, "10003", "energy", "3", "agent:tagger", "0.9");
    let shown = ok(&s, &["review", "waiting"]);
    let rows = json!([a, b, c]
        .iter()
        .map(|f| json!({ "fact": f, "token": row_token(&shown, *f) }))
        .collect::<Vec<_>>());
    // Newer proposals from the same source supersede a and c.
    fact(&s, "10001", "energy", "5", "agent:tagger", "0.9");
    fact(&s, "10003", "energy", "1", "agent:tagger", "0.9");

    let refused = accept_refused(&s, rows);

    for f in [a, c] {
        assert!(
            refused
                .message
                .contains(&format!("Fact {f} is no longer waiting")),
            "{}",
            refused.message
        );
    }
    assert!(
        !refused.message.contains(&format!("Fact {b} ")),
        "{}",
        refused.message
    );
    for track in ["10001", "10002", "10003"] {
        assert_eq!(super::review::rating(&s, track), 0, "{track}");
    }
}

#[test]
fn rows_from_two_listings_are_accepted_together_and_only_those_rows() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let first = ok(&s, &["review", "waiting"]);
    let b = fact(&s, "10002", "energy", "2", "agent:other", "0.9");
    let unchosen = fact(&s, "10003", "energy", "3", "agent:other", "0.9");
    let second = ok(&s, &["review", "waiting"]);

    let accepted = accept_rows(
        &s,
        json!([
            { "fact": a, "token": row_token(&first, a) },
            { "fact": b, "token": row_token(&second, b) },
        ]),
    )
    .unwrap();

    assert_eq!(accepted["count"], 2, "{accepted}");
    assert_eq!(super::review::rating(&s, "10001"), 4);
    assert_eq!(super::review::rating(&s, "10002"), 2);
    assert_eq!(super::review::rating(&s, "10003"), 0);
    assert_eq!(super::review::facts_waiting(&s), vec![unchosen]);
}

#[test]
fn a_row_token_replayed_after_its_row_was_accepted_or_rejected_is_refused() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let b = fact(&s, "10002", "energy", "2", "agent:tagger", "0.9");
    let shown = ok(&s, &["review", "waiting"]);
    let row_a = json!([{ "fact": a, "token": row_token(&shown, a) }]);
    let row_b = json!([{ "fact": b, "token": row_token(&shown, b) }]);
    accept_rows(&s, row_a.clone()).unwrap();
    ok(&s, &["review", "reject", &b.to_string(), "--source", "dj"]);
    // The DJ sets track 10001 again by hand; a replay must not undo it.
    ok(&s, &["tracks", "rate", "10001", "1", "--source", "dj"]);

    let replayed = accept_refused(&s, row_a);
    assert!(
        replayed
            .message
            .contains(&format!("Fact {a} is no longer waiting")),
        "{}",
        replayed.message
    );
    let replayed = accept_refused(&s, row_b);
    assert!(
        replayed
            .message
            .contains(&format!("Fact {b} is no longer waiting")),
        "{}",
        replayed.message
    );
    assert_eq!(super::review::rating(&s, "10001"), 1);
    assert_eq!(super::review::rating(&s, "10002"), 0);
}

#[test]
fn a_row_rejected_then_reopened_is_accepted_with_the_token_it_was_shown_with() {
    // Decided: acceptable. The token binds what the DJ was shown (Fact,
    // track, field, value now, proposal, source, confidence, why it
    // waits). Reopening is the DJ's own act putting that same suggestion
    // back; while every covered value is still as shown, accepting with
    // the old row is accepting exactly what the DJ saw. Any change to it
    // refuses as above. The whole-list token behaves the same.
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let shown = ok(&s, &["review", "waiting"]);
    let row = json!([{ "fact": a, "token": row_token(&shown, a) }]);
    let rejected = ok(&s, &["review", "reject", &a.to_string(), "--source", "dj"]);
    accept_refused(&s, row.clone());
    ok(
        &s,
        &[
            "review",
            "reopen",
            "--rejection",
            &rejected["rejection"]["entry"].to_string(),
            "--source",
            "dj",
        ],
    );

    let accepted = accept_rows(&s, row).unwrap();

    assert_eq!(accepted["count"], 1, "{accepted}");
    assert_eq!(super::review::rating(&s, "10001"), 4);
}

#[test]
fn rows_with_a_filter_or_all_are_refused_and_nothing_is_written() {
    let s = Sandbox::new();
    s.dj_prep_colours();
    ok(&s, &["status"]);
    let a = fact(&s, "10001", "energy", "4", "agent:tagger", "0.9");
    let b = fact(&s, "10002", "energy", "2", "agent:tagger", "0.9");
    let shown = ok(&s, &["review", "waiting"]);
    let row = json!([{ "fact": a, "token": row_token(&shown, a) }]);
    let mut client = s.client();
    for extra in [
        json!({ "all": true }),
        json!({ "field": "energy" }),
        json!({ "tracks": ["10002"] }),
        json!({ "at_least": 0.5 }),
        json!({ "from": {"kind": "agent", "name": "tagger"} }),
        json!({ "rejection": 1 }),
    ] {
        let mut args = json!({ "rows": row, "source": {"kind": "dj"} });
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        let refused = client.call::<Value>("review.accept", args).unwrap_err();
        assert_eq!(refused.code, "bad_request", "{extra}: {refused:?}");
    }
    assert_eq!(super::review::facts_waiting(&s), vec![a, b]);
    assert_eq!(super::review::rating(&s, "10001"), 0);
}
