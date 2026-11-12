//! Audio identity through the real CLI, with only temporary Libraries and WAVs.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;

use common::Sandbox;
use rbl_db::write::TrackField;
use serde_json::{json, Value};
use std::time::Duration;

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    let result = s.cli(&words);
    assert!(result.status.success(), "{args:?}: {}", result.stderr);
    result.json()
}
fn record(s: &Sandbox, id: &str) -> Vec<Value> {
    [
        ok(
            s,
            &["facts", "record", id, "genre", "Techno", "--source", "dj"],
        )["facts"][0]
            .clone(),
        ok(
            s,
            &[
                "facts",
                "record",
                id,
                "intensity",
                r#"{"summary":0.7}"#,
                "--source",
                "tool:meter",
                "--confidence",
                "1",
            ],
        )["facts"][0]
            .clone(),
    ]
    .to_vec()
}
fn change_audio(s: &Sandbox) {
    let path = s.path().join("audio/Track.wav");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[44] ^= 0x55;
    std::fs::write(path, bytes).unwrap();
}
fn describe(s: &Sandbox, id: &str) {
    s.edit(|w| {
        w.set_field(id, TrackField::Title, "Night Journey (Original Mix)")
            .unwrap();
        w.set_field(id, TrackField::Artist, "Fixture Artist")
            .unwrap();
    });
}
fn readd(s: &Sandbox, id: &str) -> String {
    let mut next = String::new();
    s.edit(|w| {
        w.delete_track(id).unwrap();
        next = w.import_file(&s.path().join("audio/Track.wav")).unwrap();
    });
    describe(s, &next);
    assert_ne!(id, next);
    next
}

#[test]
fn ordinary_reads_report_stale_measurements_but_only_judgements_stand() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    let recorded = record(&s, &id);
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    change_audio(&s);
    let shown = ok(&s, &["facts", "show", &id]);
    assert_eq!(shown["facts"], json!([recorded[0]]));
    assert_eq!(shown["stale"][0]["id"], recorded[1]["id"]);
    assert_eq!(shown["stale"][0]["stale"], true);
    let event = watcher.next_event(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(event.event, "sidecar_changed");
    assert_eq!(event.data["stale"], json!([recorded[1]["id"]]));
    ok(&s, &["facts", "show", &id]);
    assert!(watcher
        .next_event(Duration::from_millis(150))
        .unwrap()
        .is_none());
    let all = ok(&s, &["facts", "show", &id, "--all"]);
    assert_eq!(all["facts"].as_array().unwrap().len(), 2);
}

#[test]
fn a_move_and_rename_keeps_every_fact_current() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    let mut service = s.serve();
    let recorded = record(&s, &id);
    service.kill().unwrap();
    service.wait().unwrap();
    let renamed = s.path().join("audio/Renamed.mp3");
    std::fs::rename(s.path().join("audio/Track.wav"), &renamed).unwrap();
    s.edit(|w| {
        w.relocate(&id, &renamed).unwrap();
    });
    let shown = ok(&s, &["facts", "show", &id]);
    assert_eq!(shown["facts"], json!([recorded[1], recorded[0]]));
    assert_eq!(shown["stale"], json!([]));
}

#[test]
fn a_fingerprint_reattaches_facts_with_their_provenance_and_survives_restart() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    describe(&s, &id);
    let mut service = s.serve();
    let recorded = record(&s, &id);
    service.kill().unwrap();
    service.wait().unwrap();
    let next = readd(&s, &id);
    let mut service = s.serve();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let shown = ok(&s, &["facts", "show", &next]);
    let mut expected = recorded.clone();
    for fact in &mut expected {
        fact["track_id"] = json!(next);
    }
    expected.reverse();
    assert_eq!(shown["facts"], json!(expected));
    assert_eq!(shown["reattached"][0]["from_track"], id);
    assert_eq!(shown["review"], json!([]));
    let event = watcher.next_event(Duration::from_secs(2)).unwrap().unwrap();
    assert_eq!(event.event, "sidecar_changed");
    assert_eq!(event.data["kind"], "attachment");
    assert_eq!(event.data["from_track"], id);
    assert!(watcher
        .next_event(Duration::from_millis(150))
        .unwrap()
        .is_none());
    drop(watcher);
    service.kill().unwrap();
    service.wait().unwrap();
    let shown = ok(&s, &["facts", "show", &next]);
    assert_eq!(shown["facts"], json!(expected));
    // The original source can replace its re-attached answer normally.
    ok(
        &s,
        &["facts", "record", &next, "genre", "House", "--source", "dj"],
    );
    let shown = ok(&s, &["facts", "show", &next, "--kind", "genre"]);
    assert_eq!(shown["facts"].as_array().unwrap().len(), 1);
    assert_eq!(shown["facts"][0]["value"], "House");
    change_audio(&s);
    assert_eq!(
        ok(&s, &["facts", "show", &next])["stale"][0]["id"],
        recorded[1]["id"]
    );
}

#[test]
fn identifying_fields_alone_are_held_for_review_across_restarts() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    describe(&s, &id);
    let mut service = s.serve();
    record(&s, &id);
    service.kill().unwrap();
    service.wait().unwrap();
    change_audio(&s);
    let next = readd(&s, &id);
    let mut service = s.serve();
    let shown = ok(&s, &["facts", "show", &next]);
    assert_eq!(shown["facts"], json!([]));
    assert_eq!(shown["review"][0]["from_track"], id);
    assert_eq!(shown["review"][0]["reason"], "identifying_fields");
    let text = s.cli(&["facts", "show", &next]);
    assert!(text.stdout.contains("review"), "{}", text.stdout);
    service.kill().unwrap();
    service.wait().unwrap();
    assert_eq!(ok(&s, &["facts", "show", &next])["review"], shown["review"]);
}

#[test]
fn different_mixes_and_tracks_still_present_do_not_share_facts() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    describe(&s, &id);
    let mut service = s.serve();
    record(&s, &id);
    service.kill().unwrap();
    service.wait().unwrap();
    // Identical audio of a still-present track is not a re-add.
    let duplicate = s.import_track("Copy.wav", 1);
    let mut service = s.serve();
    assert_eq!(ok(&s, &["facts", "show", &duplicate])["facts"], json!([]));
    service.kill().unwrap();
    service.wait().unwrap();
    change_audio(&s);
    let next = readd(&s, &id);
    s.edit(|w| {
        w.set_field(&next, TrackField::Title, "Night Journey (Dub Mix)")
            .unwrap();
    });
    let shown = ok(&s, &["facts", "show", &next]);
    assert_eq!(shown["facts"], json!([]));
    assert_eq!(shown["review"], json!([]));
}

#[test]
fn missing_audio_never_exposes_unverified_measurements_as_standing() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    let recorded = record(&s, &id);
    std::fs::remove_file(s.path().join("audio/Track.wav")).unwrap();
    let shown = ok(&s, &["facts", "show", &id]);
    assert_eq!(shown["facts"], json!([recorded[0]]));
    assert_eq!(shown["unverified"][0]["id"], recorded[1]["id"]);
    assert_eq!(shown["unverified"][0]["stale"], false);
}

#[test]
fn several_removed_fingerprint_matches_are_held_instead_of_combining_facts() {
    let s = Sandbox::new();
    let first = s.import_track("Track.wav", 1);
    let second = s.import_track("Second.wav", 1);
    let mut service = s.serve();
    record(&s, &first);
    record(&s, &second);
    service.kill().unwrap();
    service.wait().unwrap();
    s.edit(|w| {
        w.delete_track(&second).unwrap();
    });
    let next = readd(&s, &first);
    let shown = ok(&s, &["facts", "show", &next]);
    assert_eq!(shown["facts"], json!([]));
    assert_eq!(shown["review"].as_array().unwrap().len(), 2);
    assert!(shown["review"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["reason"] == "ambiguous_fingerprint"));
}

#[test]
fn reattachment_uses_the_latest_audio_identity_not_an_old_judgements_fingerprint() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    describe(&s, &id);
    let mut service = s.serve();
    record(&s, &id);
    change_audio(&s);
    ok(&s, &["facts", "show", &id]);
    service.kill().unwrap();
    service.wait().unwrap();
    // Restore the old audio after removal. The old Judgement's original
    // fingerprint is not evidence that the track still means that mix.
    change_audio(&s);
    let next = readd(&s, &id);
    let shown = ok(&s, &["facts", "show", &next]);
    assert_eq!(shown["facts"], json!([]));
    assert_eq!(shown["review"][0]["reason"], "identifying_fields");
}

#[test]
fn stems_reattach_without_separating_again() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    let mut service = s.serve();
    let made = ok(&s, &["stems", "separate", &id]);
    service.kill().unwrap();
    service.wait().unwrap();
    let next = readd(&s, &id);
    let shown = ok(&s, &["stems", "show", &next]);
    assert_eq!(shown["stems"]["fact_id"], made["stems"]["fact_id"]);
    assert_eq!(shown["stems"]["track_id"], next);
    assert_eq!(s.engine_runs(), 1);
}

#[test]
fn a_fingerprint_match_takes_precedence_over_a_fields_only_candidate() {
    let s = Sandbox::new();
    let first = s.import_track("Track.wav", 1);
    let second = s.import_track("Second.wav", 2);
    describe(&s, &first);
    describe(&s, &second);
    let mut service = s.serve();
    let recorded = record(&s, &first);
    record(&s, &second);
    service.kill().unwrap();
    service.wait().unwrap();
    s.edit(|w| {
        w.delete_track(&second).unwrap();
    });
    let next = readd(&s, &first);
    let shown = ok(&s, &["facts", "show", &next]);
    assert_eq!(shown["facts"].as_array().unwrap().len(), 2);
    assert_eq!(shown["facts"][0]["id"], recorded[1]["id"]);
    assert_eq!(shown["review"], json!([]));
}

#[test]
fn mix_name_is_part_of_the_identifying_fields() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    describe(&s, &id);
    s.edit(|w| {
        w.library()
            .connection()
            .execute(
                "UPDATE djmdContent SET Subtitle = 'Original Mix' WHERE ID = ?1",
                [&id],
            )
            .unwrap();
    });
    let mut service = s.serve();
    record(&s, &id);
    service.kill().unwrap();
    service.wait().unwrap();
    change_audio(&s);
    let next = readd(&s, &id);
    s.edit(|w| {
        w.library()
            .connection()
            .execute(
                "UPDATE djmdContent SET Subtitle = 'Dub Mix' WHERE ID = ?1",
                [&next],
            )
            .unwrap();
    });
    let shown = ok(&s, &["facts", "show", &next]);
    assert_eq!(shown["facts"], json!([]));
    assert_eq!(shown["review"], json!([]));
}

#[test]
fn unavailable_audio_does_not_serve_stems_as_current() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    let made = ok(&s, &["stems", "separate", &id]);
    std::fs::remove_file(s.path().join("audio/Track.wav")).unwrap();
    let shown = ok(&s, &["stems", "show", &id]);
    assert_eq!(shown["stems"], Value::Null);
    assert_eq!(shown["previous"][0]["fact_id"], made["stems"]["fact_id"]);
    assert_eq!(shown["previous"][0]["stale"], false);
}

#[test]
fn attachments_follow_repeated_readds_without_copying_or_reviving_stale_facts() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    let mut service = s.serve();
    let recorded = record(&s, &id);
    change_audio(&s);
    ok(&s, &["facts", "show", &id]);
    service.kill().unwrap();
    service.wait().unwrap();
    let next = readd(&s, &id);
    let mut service = s.serve();
    let shown = ok(&s, &["facts", "show", &next]);
    assert_eq!(shown["facts"].as_array().unwrap().len(), 1);
    assert_eq!(shown["facts"][0]["id"], recorded[0]["id"]);
    assert_eq!(shown["stale"][0]["id"], recorded[1]["id"]);
    service.kill().unwrap();
    service.wait().unwrap();
    let last = readd(&s, &next);
    let shown = ok(&s, &["facts", "show", &last]);
    assert_eq!(shown["facts"][0]["id"], recorded[0]["id"]);
    assert_eq!(shown["facts"][0]["track_id"], last);
    // Returning to the original audio does not undo a stale mark.
    change_audio(&s);
    let shown = ok(&s, &["facts", "show", &last]);
    assert_eq!(shown["facts"].as_array().unwrap().len(), 1);
    assert_eq!(shown["stale"][0]["id"], recorded[1]["id"]);
}

#[test]
fn retagging_the_container_does_not_make_measurements_stale() {
    let s = Sandbox::new();
    let id = s.import_track("Track.wav", 1);
    let recorded = record(&s, &id);
    let path = s.path().join("audio/Track.wav");
    let mut bytes = std::fs::read(&path).unwrap();
    let text = b"fixture comment\0";
    let mut list = b"INFOICMT".to_vec();
    list.extend_from_slice(&(text.len() as u32).to_le_bytes());
    list.extend_from_slice(text);
    bytes.extend_from_slice(b"LIST");
    bytes.extend_from_slice(&(list.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&list);
    let length = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    std::fs::write(path, bytes).unwrap();
    let shown = ok(&s, &["facts", "show", &id]);
    assert_eq!(shown["facts"], json!([recorded[1], recorded[0]]));
    assert_eq!(shown["stale"], json!([]));
}
