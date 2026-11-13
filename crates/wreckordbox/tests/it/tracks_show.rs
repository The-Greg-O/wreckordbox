//! `wreckordbox tracks show` against a Library service on a throwaway
//! Library: the real binary, its output and its exit status.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use common::catalogue::catalogue;
use serde_json::json;

#[test]
fn tracks_show_as_json_is_every_field_of_the_track_under_stable_names() {
    let library = catalogue();

    let run = library.sandbox.cli(&["--json", "tracks", "show", "10003"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    assert_eq!(
        run.json(),
        json!({
            "id": "10003",
            "title": "Midnight Drive",
            "artist": "Ada Lovelace",
            "album": "Night Moves",
            "genre": "Tech House",
            "label": "Analytical",
            "year": 2021,
            "bpm": 128.03,
            "key": "8A",
            "energy": 4,
            "rating": 4,
            "prep_colour": "Green",
            "colour": "Green",
            "colour_id": 5,
            "comment": "needs an intro edit",
            "duration_seconds": 300,
            "date_added": null,
            "play_count": 7,
            "location": "/fixture/audio/track003.mp3",
            "file_name": "track003.mp3",
            "analysed": true,
            "my_tags": [{ "id": "700002", "category": "Situation", "tag": "Peak" }],
            "playlists": [
                { "id": "900000", "name": "Playlist 0" },
                { "id": library.warm_up_set, "name": "Warm up set" },
                { "id": run.json()["playlists"][2]["id"], "name": "House" }
            ]
        })
    );
}

#[test]
fn a_track_with_no_energy_or_prep_colour_says_so_beside_its_rating_and_colour() {
    let library = catalogue();

    let track = library
        .sandbox
        .cli(&["--json", "tracks", "show", "10020"])
        .json();

    assert_eq!(track["energy"], json!(null));
    assert_eq!(track["rating"], 0);
    assert_eq!(track["prep_colour"], json!(null));
    assert_eq!(track["colour"], json!(null));
    assert_eq!(track["colour_id"], 0);
    assert_eq!(track["year"], json!(null));
    assert_eq!(track["my_tags"], json!([]));
    assert_eq!(track["playlists"], json!([]));
}

#[test]
fn an_imported_track_shows_when_it_was_added_and_that_it_is_not_analysed() {
    let library = catalogue();

    let track = library
        .sandbox
        .cli(&["--json", "tracks", "show", &library.fresh])
        .json();

    assert_eq!(track["id"], json!(library.fresh));
    assert_eq!(track["analysed"], false);
    assert_eq!(track["bpm"], json!(null));
    assert_eq!(track["duration_seconds"], 95);
    assert_eq!(track["file_name"], "Fresh Cut.wav");
    let location = track["location"].as_str().unwrap();
    assert!(location.ends_with("/audio/Fresh Cut.wav"), "{location}");
    let added = track["date_added"].as_str().unwrap();
    let shape: String = added
        .chars()
        .map(|c| if c.is_ascii_digit() { '9' } else { c })
        .collect();
    assert_eq!(shape, "9999-99-99", "{added}");
}

#[test]
fn tracks_show_prints_the_track_a_field_to_a_line() {
    let library = catalogue();

    let run = library.sandbox.cli(&["tracks", "show", "10003"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    let lines: Vec<(&str, &str)> = run
        .stdout
        .lines()
        .map(|line| {
            let (name, value) = line.split_once("  ").expect(line);
            (name.trim(), value.trim())
        })
        .collect();
    assert_eq!(
        lines,
        [
            ("ID", "10003"),
            ("Title", "Midnight Drive"),
            ("Artist", "Ada Lovelace"),
            ("Album", "Night Moves"),
            ("Genre", "Tech House"),
            ("Label", "Analytical"),
            ("Year", "2021"),
            ("BPM", "128.03"),
            ("Key", "8A"),
            ("Energy", "4"),
            ("Rating", "4 stars"),
            ("Prep colour", "Green"),
            ("Colour", "Green (5)"),
            ("Comment", "needs an intro edit"),
            ("Duration", "5:00"),
            ("Date added", "-"),
            ("Play count", "7"),
            ("Location", "/fixture/audio/track003.mp3"),
            ("Analysed", "yes"),
            ("MyTags", "Situation: Peak"),
            ("Playlists", "Playlist 0, Warm up set, House"),
        ]
    );
}

#[test]
fn showing_a_track_the_library_does_not_hold_fails_and_says_which() {
    let library = catalogue();

    let run = library.sandbox.cli(&["--json", "tracks", "show", "424242"]);

    assert!(!run.status.success());
    assert_eq!(run.stdout, "");
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "track_not_found");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("424242"),
        "{error}"
    );

    let by_hand = library.sandbox.cli(&["tracks", "show", "424242"]);
    assert!(!by_hand.status.success());
    assert_eq!(
        by_hand.stderr,
        "wreckordbox: there is no track 424242 in the Library\n"
    );
}
