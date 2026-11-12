//! Library Protection through the real `wreckordbox` command and a service
//! the command started by itself, on a throwaway Library: while protection
//! is on nothing that changes the Library is done, and reading is untouched.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use common::Sandbox;
use rbl_db::fixture;

/// The window's own words for a change refused by Library Protection
/// (`refusal(true)` in `src/lib/menu.ts`).
const MESSAGE: &str =
    "Editing is locked by Library Protection. Turn it off in Preferences to edit.";

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

/// The identifiers of the MyTags on a track.
fn tags_of(sandbox: &Sandbox, id: &str) -> Vec<String> {
    let run = sandbox.cli(&["--json", "mytags", "show", id]);
    assert!(run.status.success(), "{}", run.stderr);
    run.json()["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| tag["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_rating_is_refused_while_protection_is_on_and_nothing_changes() {
    let sandbox = Sandbox::new();
    sandbox.protect(true);

    let run = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);

    assert!(!run.status.success());
    assert_eq!(run.stdout, "");
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "library_protected");
    assert_eq!(error["error"]["message"], MESSAGE);
    assert_eq!(listed_rating(&sandbox, "10003"), 0);
}

#[test]
fn the_same_rating_succeeds_once_protection_is_off() {
    let sandbox = Sandbox::new();
    sandbox.protect(true);
    assert!(!sandbox
        .cli(&["tracks", "rate", "10003", "4"])
        .status
        .success());

    // The service reads the switch before every edit: no restart.
    sandbox.protect(false);
    let run = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.json()["rating"], 4);
    assert_eq!(listed_rating(&sandbox, "10003"), 4);
}

#[test]
fn a_service_with_nothing_recorded_treats_protection_as_on() {
    // No window has ever run here and nobody has turned protection off.
    let sandbox = Sandbox::untouched();

    let run = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);

    assert!(!run.status.success());
    assert_eq!(run.error_json()["error"]["code"], "library_protected");
    assert_eq!(listed_rating(&sandbox, "10003"), 0);
}

#[test]
fn a_switch_file_that_cannot_be_read_counts_as_on() {
    let sandbox = Sandbox::new();
    std::fs::write(sandbox.protection_file(), "not json").unwrap();

    let run = sandbox.cli(&["--json", "tracks", "rate", "10003", "4"]);

    assert_eq!(run.error_json()["error"]["code"], "library_protected");
}

#[test]
fn every_mytag_edit_is_refused_while_protection_is_on_and_nothing_changes() {
    let sandbox = Sandbox::new();
    let (category, peak, warm_up) = (
        fixture::MY_TAG_CATEGORY,
        fixture::MY_TAG_PEAK,
        fixture::MY_TAG_WARM_UP,
    );
    sandbox.detail_track(
        1,
        &common::Details {
            my_tags: &[peak],
            ..Default::default()
        },
    );
    let catalogue = sandbox.cli(&["--json", "mytags", "list"]).json();
    sandbox.protect(true);

    let edits: [&[&str]; 8] = [
        &["mytags", "create", category, "Closing"],
        &["mytags", "rename", peak, "Summit"],
        &["mytags", "move", peak, "2"],
        &["mytags", "reorder", warm_up, peak],
        &["mytags", "merge", peak, warm_up],
        &["mytags", "delete", warm_up],
        &["mytags", "assign", warm_up, "10002"],
        &["mytags", "unassign", peak, "10001"],
    ];
    for edit in edits {
        let mut args = vec!["--json"];
        args.extend_from_slice(edit);
        let run = sandbox.cli(&args);
        assert!(!run.status.success(), "{edit:?} was not refused");
        let error = run.error_json();
        assert_eq!(error["error"]["code"], "library_protected", "{edit:?}");
        assert_eq!(error["error"]["message"], MESSAGE, "{edit:?}");
    }

    assert_eq!(sandbox.cli(&["--json", "mytags", "list"]).json(), catalogue);
    assert_eq!(tags_of(&sandbox, "10001"), [peak]);
    assert_eq!(tags_of(&sandbox, "10002"), Vec::<String>::new());
}

#[test]
fn a_mytag_is_put_on_a_track_once_protection_is_off() {
    let sandbox = Sandbox::new();
    let peak = fixture::MY_TAG_PEAK;
    sandbox.protect(true);
    assert!(!sandbox
        .cli(&["mytags", "assign", peak, "10002"])
        .status
        .success());

    sandbox.protect(false);
    let run = sandbox.cli(&["--json", "mytags", "assign", peak, "10002"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.json()["changed"], 1);
    assert_eq!(tags_of(&sandbox, "10002"), [peak]);
}

/// The name of a track's Prep colour, `null` when it has none.
fn prep_colour(sandbox: &Sandbox, id: &str) -> serde_json::Value {
    let run = sandbox.cli(&["--json", "tracks", "show", id]);
    assert!(run.status.success(), "{}", run.stderr);
    run.json()["prep_colour"].clone()
}

#[test]
fn a_prep_colour_is_refused_while_protection_is_on_and_nothing_changes() {
    let sandbox = Sandbox::new();
    sandbox.dj_prep_colours();
    sandbox.protect(true);

    let run = sandbox.cli(&["--json", "intake", "colour", "10003", "Clean"]);

    assert!(!run.status.success());
    assert_eq!(run.stdout, "");
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "library_protected");
    assert_eq!(error["error"]["message"], MESSAGE);
    assert_eq!(prep_colour(&sandbox, "10003"), serde_json::Value::Null);
    // Still waiting in Intake, which reads as ever.
    let intake = sandbox.cli(&["--json", "intake", "list"]).json();
    assert!(intake["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|track| track["id"] == "10003"));
}

#[test]
fn the_same_prep_colour_is_set_once_protection_is_off() {
    let sandbox = Sandbox::new();
    sandbox.dj_prep_colours();
    sandbox.protect(true);
    assert!(!sandbox
        .cli(&["intake", "colour", "10003", "Clean"])
        .status
        .success());

    sandbox.protect(false);
    let run = sandbox.cli(&["--json", "intake", "colour", "10003", "Clean"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.json()["changed"], true);
    assert_eq!(prep_colour(&sandbox, "10003"), "Green");
}

#[test]
fn reading_works_whether_or_not_protection_is_on() {
    let sandbox = Sandbox::new();
    sandbox.protect(true);

    let reads: [&[&str]; 8] = [
        &["status"],
        &["tracks", "list"],
        &["tracks", "show", "10001"],
        &["tracks", "query", "--search", "Track"],
        &["mytags", "list"],
        &["mytags", "show", "10001"],
        &["playlists", "tree"],
        &["history", "list"],
    ];
    for read in reads {
        let run = sandbox.cli(read);
        assert!(run.status.success(), "{read:?}: {}", run.stderr);
    }
}

#[test]
fn the_command_says_whether_protection_is_on_and_turns_it_off_and_on() {
    let sandbox = Sandbox::untouched();

    let shown = sandbox.cli(&["--json", "protection"]);
    assert!(shown.status.success(), "{}", shown.stderr);
    assert_eq!(shown.json()["protect_library"], true);
    assert_eq!(
        sandbox.cli(&["protection"]).stdout,
        "Library Protection is on: nothing can change the Library.\n"
    );

    let off = sandbox.cli(&["--json", "protection", "off"]);
    assert!(off.status.success(), "{}", off.stderr);
    assert_eq!(
        off.json(),
        serde_json::json!({ "protect_library": false, "previous": true })
    );
    assert_eq!(
        sandbox.cli(&["protection"]).stdout,
        "Library Protection is off: the Library can be changed.\n"
    );
    // What the window reads when it next starts.
    assert_eq!(
        std::fs::read_to_string(sandbox.protection_file()).unwrap(),
        r#"{"protect_library":false}"#
    );
    assert!(sandbox
        .cli(&["tracks", "rate", "10003", "4"])
        .status
        .success());

    let on = sandbox.cli(&["protection", "on"]);
    assert_eq!(on.stdout, "Library Protection is now on.\n");
    assert_eq!(
        sandbox.cli(&["protection", "on"]).stdout,
        "Library Protection was already on.\n"
    );
    let refused = sandbox.cli(&["tracks", "rate", "10003", "5"]);
    assert!(!refused.status.success());
    assert_eq!(listed_rating(&sandbox, "10003"), 4);
}

#[test]
fn by_hand_a_refusal_says_how_to_turn_protection_off_from_the_command() {
    let sandbox = Sandbox::new();
    sandbox.protect(true);

    let run = sandbox.cli(&["tracks", "rate", "10003", "4"]);

    assert_eq!(
        run.stderr,
        format!("wreckordbox: {MESSAGE}\nOr turn it off here: wreckordbox protection off\n")
    );
}
