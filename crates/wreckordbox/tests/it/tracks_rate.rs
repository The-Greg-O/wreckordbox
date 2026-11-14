//! `wreckordbox tracks rate` against a Library service on a throwaway
//! Library: the real binary, its output and exit status, and what a second
//! connected client is told.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use std::time::Duration;

use common::Sandbox;
use serde_json::json;

/// The rating `tracks list` shows for the track with `id`.
fn listed_rating(sandbox: &Sandbox, id: &str) -> serde_json::Value {
    let run = sandbox.cli(&["--json", "tracks", "list"]);
    assert!(run.status.success(), "{}", run.stderr);
    let list = run.json();
    let track = list["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .expect(&run.stdout);
    track["rating"].clone()
}

#[test]
fn rating_a_track_reports_the_old_and_new_rating_and_the_next_list_shows_it() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);

    let run = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    let rated = run.json();
    assert_eq!(rated["id"], "10003");
    assert_eq!(rated["title"], "Midnight Drive");
    assert_eq!(rated["previous_rating"], 2);
    assert_eq!(rated["rating"], 4);
    assert_eq!(listed_rating(&sandbox, "10003"), 4);
    // No other track was touched.
    assert_eq!(listed_rating(&sandbox, "10004"), 0);
}

#[test]
fn by_hand_the_result_is_one_readable_line_and_the_run_it_is_in() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 1);

    let run = sandbox.cli(&["tracks", "rate", "10003", "5"]);
    assert!(run.status.success(), "{}", run.stderr);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "Rated track 10003 \"Midnight Drive\": 5 stars, was 1 star."
    );
    assert_eq!(lines.len(), 2, "{}", run.stdout);
    assert!(
        lines[1].starts_with("Run ") && lines[1].ends_with("(tool:wreckordbox)."),
        "{}",
        run.stdout
    );
    assert_eq!(run.stderr, "");

    // Rating it the same again succeeds and says so.
    let again = sandbox.cli(&["tracks", "rate", "10003", "5"]);
    assert!(again.status.success(), "{}", again.stderr);
    assert_eq!(
        again.stdout,
        "Track 10003 \"Midnight Drive\" already has 5 stars.\n"
    );

    // 0 clears the rating.
    let cleared = sandbox
        .cli(&["--json", "tracks", "rate", "10003", "0"])
        .json();
    assert_eq!(
        (
            cleared["previous_rating"].clone(),
            cleared["rating"].clone()
        ),
        (json!(5), json!(0))
    );
    assert_eq!(listed_rating(&sandbox, "10003"), 0);
}

#[test]
fn a_second_client_is_told_what_kind_of_data_changed_and_the_new_generation() {
    let sandbox = Sandbox::new();
    sandbox.status();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let first = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);
    assert!(first.status.success(), "{}", first.stderr);

    let event = watcher
        .next_event(Duration::from_secs(5))
        .unwrap()
        .expect("no event arrived");
    assert_eq!(event.event, "library_changed");
    assert_eq!(event.data["kind"], "library");
    let generation = event.data["generation"]
        .as_u64()
        .expect("a generation number");
    // The edit's own answer names the same generation.
    assert_eq!(first.json()["generation"], generation);
    // Exactly what the event carries, so a new field is a decision.
    assert_eq!(
        event.data,
        json!({ "kind": "library", "generation": generation })
    );
    // One edit, one event: nothing more before the ordered barrier.
    common::barrier::told_nothing(&mut watcher);

    // The next edit is a newer generation.
    let second = sandbox.cli(&["tracks", "rate", "10004", "1"]);
    assert!(second.status.success(), "{}", second.stderr);
    let event = watcher
        .next_event(Duration::from_secs(5))
        .unwrap()
        .expect("no second event arrived");
    assert!(
        event.data["generation"].as_u64().unwrap() > generation,
        "{event:?}"
    );
}

#[test]
fn an_unknown_track_fails_and_changes_nothing() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);
    let before = sandbox.cli(&["--json", "tracks", "list"]).json();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let run = sandbox.cli(&["--json", "tracks", "rate", "99999", "4"]);

    assert_eq!(run.status.code(), Some(1));
    assert_eq!(run.stdout, "");
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "track_not_found");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("99999"),
        "{error}"
    );
    assert_eq!(sandbox.cli(&["--json", "tracks", "list"]).json(), before);
    common::barrier::told_nothing(&mut watcher);

    // Not a number at all is the same failure, and says so in words by hand.
    let text = sandbox.cli(&["tracks", "rate", "midnight", "4"]);
    assert_eq!(text.status.code(), Some(1));
    assert_eq!(text.stdout, "");
    assert_eq!(
        text.stderr,
        "wreckordbox: there is no track midnight in the Library\n"
    );
}

#[test]
fn a_rating_out_of_range_is_refused() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);
    let mut watcher = {
        sandbox.status();
        sandbox.client()
    };
    watcher.subscribe().unwrap();

    let run = sandbox.cli(&["--json", "tracks", "rate", "10003", "6"]);
    assert_eq!(run.status.code(), Some(1));
    assert_eq!(run.stdout, "");
    assert_eq!(run.error_json()["error"]["code"], "invalid_rating");

    // The service refuses it from any client, not only the command.
    let direct = watcher
        .call::<serde_json::Value>("tracks.rate", json!({ "id": "10003", "rating": 6 }))
        .unwrap_err();
    assert_eq!(direct.code, "invalid_rating");
    // Not a number of stars at all never reaches the service.
    for bad in ["-1", "five", "300"] {
        let run = sandbox.cli(&["tracks", "rate", "10003", bad]);
        assert!(!run.status.success(), "{bad}");
        assert_eq!(run.stdout, "", "{bad}");
    }

    assert_eq!(listed_rating(&sandbox, "10003"), 2);
    common::barrier::told_nothing(&mut watcher);
}

#[test]
fn a_real_library_is_not_written_to_under_test() {
    let sandbox = Sandbox::new();
    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 2);
    // Without the marker that says this Library is a fixture, it is located
    // as the installed one, and the service inherits that from the command
    // that starts it.
    let real = |args: &[&str]| {
        sandbox.run(
            sandbox
                .command()
                .env_remove("RBXPORT_TEST_FIXTURE")
                .args(args),
        )
    };
    assert!(real(&["--json", "status"]).status.success());
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();

    let run = real(&["--json", "tracks", "rate", "10003", "4"]);

    assert_eq!(run.status.code(), Some(1));
    assert_eq!(run.stdout, "");
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "write_refused");
    // The write guard's own words.
    assert_eq!(
        error["error"]["message"],
        "RBXPORT_TEST is set and this is the real library; tests must copy a fixture first"
    );
    assert_eq!(listed_rating(&sandbox, "10003"), 2);
    common::barrier::told_nothing(&mut watcher);
}

#[test]
fn a_rating_outlives_the_service_that_made_it() {
    let sandbox = Sandbox::new();
    let run = sandbox.cli(&["tracks", "rate", "10003", "3"]);
    assert!(run.status.success(), "{}", run.stderr);

    let pid = sandbox.status()["pid"].as_u64().unwrap();
    std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
        .unwrap();
    assert!((0..200).any(|_| {
        std::thread::sleep(Duration::from_millis(25));
        !common::alive(pid)
    }));

    // A new service reads it from the Library itself.
    assert_ne!(sandbox.status()["pid"], pid);
    assert_eq!(listed_rating(&sandbox, "10003"), 3);
}

#[test]
fn the_command_describes_itself() {
    let sandbox = Sandbox::new();
    let help = sandbox.cli(&["tracks", "rate", "--help"]);
    assert!(help.status.success());
    assert!(
        help.stdout.contains("<ID>") && help.stdout.contains("<STARS>"),
        "{}",
        help.stdout
    );
    assert!(help.stdout.contains("0 to 5"), "{}", help.stdout);
}
