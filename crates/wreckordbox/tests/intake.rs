//! `wreckordbox intake`: the Intake and Problems queues, asked of a service
//! on a fixture Library through the real command.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use common::{Details, Sandbox};
use rbl_db::fixture::{self, Shape};
use serde_json::{json, Value};

fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.json()
}

fn text(sandbox: &Sandbox, args: &[&str]) -> String {
    let run = sandbox.cli(args);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.stdout
}

fn ids(page: &Value) -> Vec<&str> {
    page["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].as_str().unwrap())
        .collect()
}

/// Four tracks: the first untouched, the second fully prepared but for its
/// Prep colour, the third marked Clean, the fourth marked Problem.
fn prepared() -> Sandbox {
    let sandbox = Sandbox::with_shape(Shape {
        tracks: 4,
        ..Shape::default()
    });
    sandbox.dj_prep_colours();
    sandbox.describe_track(1, "Ready", "Someone", "8A", 4);
    sandbox.detail_track(
        1,
        &Details {
            my_tags: &[fixture::MY_TAG_PEAK],
            ..Details::default()
        },
    );
    sandbox.edit(|writer| {
        let ready = fixture::track_id(1);
        writer.add_cue(&ready, 1, 12_000).unwrap();
        writer.set_analysis_lock(&ready, true).unwrap();
    });
    sandbox.detail_track(
        2,
        &Details {
            colour_id: Some(5),
            ..Details::default()
        },
    );
    sandbox.detail_track(
        3,
        &Details {
            colour_id: Some(2),
            ..Details::default()
        },
    );
    sandbox
}

#[test]
fn intake_list_shows_the_tracks_with_no_prep_colour_and_what_each_needs() {
    let sandbox = prepared();

    let page = ok(&sandbox, &["intake", "list"]);
    assert_eq!(ids(&page), ["10000", "10001"]);
    assert_eq!(page["total"], 2);
    assert_eq!(
        page["tracks"][0]["needs"],
        json!(["energy", "tags", "hot_cues", "grid"])
    );
    assert_eq!(
        page["tracks"][0]["empty_dimensions"][0]["name"],
        "Situation"
    );
    assert_eq!(page["tracks"][0]["grid"], "not_confirmed");
    assert_eq!(page["tracks"][1]["needs"], json!([]));
    assert_eq!(page["tracks"][1]["artist"], "Someone");

    let table = text(&sandbox, &["intake", "list"]);
    let lines: Vec<Vec<&str>> = table
        .lines()
        .map(|line| {
            line.split("  ")
                .map(str::trim)
                .filter(|cell| !cell.is_empty())
                .collect()
        })
        .collect();
    assert_eq!(
        lines,
        [
            vec!["ID", "TITLE", "ARTIST", "BPM", "KEY", "NEEDS"],
            vec![
                "10000",
                "Track 000",
                "-",
                "128.00",
                "-",
                "energy, tags (Situation), hot_cues, grid (not confirmed)"
            ],
            vec!["10001", "Ready", "Someone", "128.01", "8A", "-"],
        ]
    );
}

#[test]
fn a_queue_is_printed_a_page_at_a_time() {
    let sandbox = Sandbox::with_shape(Shape {
        tracks: 5,
        ..Shape::default()
    });

    let first = ok(&sandbox, &["intake", "list", "--limit", "2"]);
    assert_eq!(ids(&first), ["10000", "10001"]);
    assert_eq!(first["next_offset"], 2);
    let last = ok(
        &sandbox,
        &["intake", "list", "--limit", "2", "--offset", "4"],
    );
    assert_eq!(ids(&last), ["10004"]);
    assert_eq!(last["next_offset"], Value::Null);

    let table = text(&sandbox, &["intake", "list", "--limit", "2"]);
    assert_eq!(
        table.lines().last().unwrap(),
        "Tracks 1 to 2 of 5; next page: --offset 2"
    );
    let table = text(
        &sandbox,
        &["intake", "list", "--limit", "2", "--offset", "4"],
    );
    assert_eq!(table.lines().last().unwrap(), "Tracks 5 to 5 of 5.");
}

#[test]
fn intake_problems_shows_the_tracks_marked_problem() {
    let sandbox = prepared();

    let page = ok(&sandbox, &["intake", "problems"]);
    assert_eq!(ids(&page), ["10003"]);
    assert_eq!(
        page["prep_colour"],
        json!({ "id": 2, "colour": "Red", "label": "Problem" })
    );

    let table = text(&sandbox, &["intake", "problems"]);
    assert!(
        table
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("10003  Track 003"),
        "{table}"
    );
}

#[test]
fn setting_a_prep_colour_moves_a_track_out_of_intake_and_into_problems() {
    let sandbox = prepared();
    let mut watcher = {
        sandbox.status();
        sandbox.client()
    };
    watcher.subscribe().unwrap();

    let line = text(&sandbox, &["intake", "colour", "10000", "clean"]);
    assert_eq!(
        common::without_run(&line).trim_end(),
        "Track 10000 \"Track 000\": Prep colour Clean (Green), was none."
    );
    let event = watcher
        .next_event(std::time::Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(event.event, "library_changed");
    assert_eq!(ids(&ok(&sandbox, &["intake", "list"])), ["10001"]);
    assert_eq!(
        ok(&sandbox, &["tracks", "show", "10000"])["prep_colour"],
        "Green"
    );

    let marked = ok(&sandbox, &["intake", "colour", "10001", "Problem"]);
    assert_eq!(marked["prep_colour"]["colour"], "Red");
    assert_eq!(marked["previous"], Value::Null);
    assert_eq!(ids(&ok(&sandbox, &["intake", "list"])), Vec::<&str>::new());
    assert_eq!(
        ids(&ok(&sandbox, &["intake", "problems"])),
        ["10001", "10003"]
    );

    assert_eq!(
        text(&sandbox, &["intake", "colour", "10001", "red"]).trim_end(),
        "Track 10001 \"Ready\" already has Prep colour Problem (Red)."
    );
    assert_eq!(
        common::without_run(&text(&sandbox, &["intake", "colour", "10001", "none"])).trim_end(),
        "Track 10001 \"Ready\": no Prep colour, was Problem (Red)."
    );
    assert_eq!(
        text(&sandbox, &["intake", "list"]).lines().count(),
        2,
        "the track is back in Intake"
    );
    assert_eq!(
        text(&sandbox, &["intake", "problems", "--offset", "5"]).trim_end(),
        "No tracks that far in: the queue holds 1."
    );
}

#[test]
fn intake_count_says_how_many_wait_in_each_queue() {
    let sandbox = prepared();

    let counts = ok(&sandbox, &["intake", "count"]);
    assert_eq!(counts["intake"], 2);
    assert_eq!(counts["problems"], 1);

    assert_eq!(
        text(&sandbox, &["intake", "count"]).trim_end(),
        "Intake    2\nProblems  1  (Prep colour Problem, Red)"
    );
}

#[test]
fn problems_is_refused_clearly_when_no_colour_label_is_named_problem() {
    let sandbox = Sandbox::with_shape(Shape {
        tracks: 2,
        ..Shape::default()
    });
    sandbox.name_colours(&[(4, "Kaputt")]);
    sandbox.detail_track(
        1,
        &Details {
            colour_id: Some(4),
            ..Details::default()
        },
    );

    let refused = sandbox.cli(&["--json", "intake", "problems"]);
    assert!(!refused.status.success());
    assert_eq!(refused.stdout, "");
    let error = refused.error_json();
    assert_eq!(error["error"]["code"], "problem_colour_not_found");

    let plain = sandbox.cli(&["intake", "problems"]);
    assert!(!plain.status.success());
    assert!(plain.stderr.contains("Kaputt (Yellow)"), "{}", plain.stderr);

    // Intake is still counted, and the count says why Problems is not.
    let counts = ok(&sandbox, &["intake", "count"]);
    assert_eq!(counts["intake"], 1);
    assert_eq!(counts["problems"], Value::Null);
    let lines = text(&sandbox, &["intake", "count"]);
    assert!(lines.starts_with("Intake    1\nProblems  -  ("), "{lines}");

    // The DJ's own word for it, on the command.
    let page = ok(&sandbox, &["intake", "problems", "--label", "kaputt"]);
    assert_eq!(ids(&page), ["10001"]);
    assert_eq!(
        ok(&sandbox, &["intake", "count", "--label", "Kaputt"])["problems"],
        1
    );

    let unknown = sandbox.cli(&["--json", "intake", "colour", "10000", "Mauve"]);
    assert!(!unknown.status.success());
    assert_eq!(
        unknown.error_json()["error"]["code"],
        "prep_colour_not_found"
    );
    assert_eq!(ok(&sandbox, &["intake", "count"])["intake"], 1);
}

#[test]
fn the_name_that_means_problem_can_be_set_for_the_service() {
    let sandbox = Sandbox::with_shape(Shape {
        tracks: 2,
        ..Shape::default()
    });
    sandbox.name_colours(&[(4, "Kaputt")]);
    sandbox.detail_track(
        0,
        &Details {
            colour_id: Some(4),
            ..Details::default()
        },
    );

    // The first command starts the service, which is what reads it.
    let started = sandbox.run(
        sandbox
            .command()
            .env("WRECKORDBOX_PROBLEM_LABEL", "Kaputt")
            .args(["--json", "intake", "problems"]),
    );
    assert!(started.status.success(), "{}", started.stderr);
    assert_eq!(ids(&started.json()), ["10000"]);
    assert_eq!(ids(&ok(&sandbox, &["intake", "problems"])), ["10000"]);
}

#[test]
fn a_title_an_agent_proposed_shows_as_needed_until_the_dj_records_one() {
    let sandbox = Sandbox::with_shape(Shape {
        tracks: 1,
        ..Shape::default()
    });
    // The DJ declares the kind, as any kind not built in is declared.
    std::fs::create_dir_all(sandbox.sidecar_dir()).unwrap();
    std::fs::write(
        sandbox.sidecar_dir().join("kinds.toml"),
        "[kinds.title]\nnature = \"judgement\"\nvalue = \"text\"\n",
    )
    .unwrap();
    ok(
        &sandbox,
        &[
            "facts",
            "record",
            "10000",
            "title",
            "A Better Title",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.7",
        ],
    );

    let page = ok(&sandbox, &["intake", "list"]);
    assert_eq!(page["tracks"][0]["needs"][0], "title");
    assert_eq!(
        page["tracks"][0]["suggestions"][0]["value"],
        "A Better Title"
    );
    assert!(
        text(&sandbox, &["intake", "list"]).contains("title (\"A Better Title\" proposed), energy"),
    );

    ok(
        &sandbox,
        &[
            "facts",
            "record",
            "10000",
            "title",
            "Track 000",
            "--source",
            "dj",
        ],
    );
    let page = ok(&sandbox, &["intake", "list"]);
    assert_eq!(page["tracks"][0]["needs"][0], "energy");
    assert_eq!(page["tracks"][0]["suggestions"], json!([]));
}
