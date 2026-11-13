//! The Track Filter's fields on `wreckordbox tracks query` (wbx-57): several
//! keys, several Prep colours, and MyTags picked within a category with the
//! category's AND / OR switch, as the window's Track Filter bar picks them.
//! The real binary against a service on a throwaway Library.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use common::catalogue::{catalogue, Catalogue};
use common::Sandbox;
use rbl_db::fixture;
use serde_json::{json, Value};

fn page(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json", "tracks", "query"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--limit", "1000"]);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    assert_eq!(run.stderr, "");
    run.json()
}

fn ids_of(page: &Value) -> Vec<String> {
    page["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The identifiers `tracks query <args>` matches.
fn ids(sandbox: &Sandbox, args: &[&str]) -> Vec<String> {
    page(sandbox, args)["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The error code `tracks query --json <args>` fails with, after checking
/// it printed nothing.
fn refusal(sandbox: &Sandbox, args: &[&str]) -> String {
    let mut all = vec!["--json", "tracks", "query"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(!run.status.success(), "{args:?} succeeded: {}", run.stdout);
    assert_eq!(run.stdout, "");
    run.error_json()["error"]["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn several_keys_match_a_track_in_any_of_them() {
    let Catalogue { sandbox, .. } = catalogue();

    assert_eq!(
        ids(&sandbox, &["--key", "8A", "--key", "11B"]),
        ["10003", "10007", "10012"]
    );
    // One key is what it always was.
    assert_eq!(ids(&sandbox, &["--key", "11B"]), ["10007"]);
    // A key no track is in narrows to nothing, as the bar's would.
    assert_eq!(ids(&sandbox, &["--key", "1A"]), Vec::<String>::new());
}

#[test]
fn several_prep_colours_match_a_track_in_any_of_them() {
    let Catalogue { sandbox, .. } = catalogue();

    assert_eq!(
        ids(
            &sandbox,
            &["--prep-colour", "green", "--prep-colour", "Red"]
        ),
        ["10003", "10007"]
    );
    assert_eq!(
        refusal(
            &sandbox,
            &["--prep-colour", "green", "--prep-colour", "mauve"]
        ),
        "bad_request"
    );
}

#[test]
fn mytags_picked_in_a_category_need_every_one_unless_the_category_is_any() {
    let Catalogue { sandbox, .. } = catalogue();
    let peak_and_warm_up = [
        "--mytag-in",
        "Situation",
        "Peak",
        "--mytag-in",
        "Situation",
        "Warm-up",
    ];

    assert_eq!(
        ids(&sandbox, &["--mytag-in", "Situation", "Peak"]),
        ["10003", "10012"]
    );
    assert_eq!(ids(&sandbox, &peak_and_warm_up), Vec::<String>::new());
    assert_eq!(
        ids(
            &sandbox,
            &[&peak_and_warm_up[..], &["--mytag-any", "Situation"]].concat()
        ),
        ["10003", "10007", "10012"]
    );
    assert_eq!(
        ids(
            &sandbox,
            &[
                "--mytag-in",
                "Situation",
                "Peak",
                "--mytag-in",
                "Situation",
                "Vocal"
            ]
        ),
        ["10012"]
    );
}

#[test]
fn a_mytag_picked_in_one_category_does_not_match_the_same_name_in_another() {
    let Catalogue { sandbox, .. } = catalogue();
    sandbox.four_categories();
    // "Peak" is also a Sound, carried by track 10007 only.
    let sound_peak = sandbox.create_my_tag_in("700102", "Peak");
    sandbox.edit(|writer| {
        writer
            .set_my_tags(
                &fixture::track_id(7),
                &[fixture::MY_TAG_WARM_UP.to_owned(), sound_peak.clone()],
            )
            .unwrap();
    });

    assert_eq!(
        ids(&sandbox, &["--mytag-in", "Situation", "Peak"]),
        ["10003", "10012"]
    );
    assert_eq!(ids(&sandbox, &["--mytag-in", "Sound", "Peak"]), ["10007"]);
    // Two categories must both agree.
    assert_eq!(
        ids(
            &sandbox,
            &[
                "--mytag-in",
                "Situation",
                "Warm-up",
                "--mytag-in",
                "Sound",
                "Peak"
            ]
        ),
        ["10007"]
    );
}

#[test]
fn a_category_or_tag_the_library_no_longer_holds_is_left_out_with_a_note() {
    let Catalogue { sandbox, .. } = catalogue();

    // "Sunset" is not a Situation: the pick keeps Peak and says so.
    let found = page(
        &sandbox,
        &[
            "--mytag-in",
            "Situation",
            "Peak",
            "--mytag-in",
            "Situation",
            "Sunset",
            "--mytag-any",
            "Situation",
        ],
    );
    assert_eq!(ids_of(&found), ["10003", "10012"]);
    assert_eq!(found["dropped"], json!(["Situation: Sunset"]));

    // A category that is gone takes its picks with it: no constraint is left.
    let found = page(&sandbox, &["--mytag-in", "Weather", "Rain", "--key", "11B"]);
    assert_eq!(ids_of(&found), ["10007"]);
    assert_eq!(found["dropped"], json!(["Weather"]));

    // Said in the text output too.
    let run = sandbox.cli(&[
        "tracks",
        "query",
        "--mytag-in",
        "Weather",
        "Rain",
        "--key",
        "11B",
    ]);
    assert!(run.status.success(), "{}", run.stderr);
    assert!(
        run.stdout
            .contains("Left out of the filter, no longer in the Library: Weather"),
        "{}",
        run.stdout
    );

    // Nothing dropped, nothing said.
    assert_eq!(page(&sandbox, &["--key", "11B"]).get("dropped"), None);
}

#[test]
fn a_mode_with_no_tags_is_refused() {
    let Catalogue { sandbox, .. } = catalogue();
    assert_eq!(
        refusal(&sandbox, &["--mytag-any", "Situation"]),
        "bad_request"
    );
}

#[test]
fn a_category_and_tag_with_colons_in_their_names_round_trip() {
    let Catalogue { sandbox, .. } = catalogue();
    let [_, mood, _, _] = sandbox.four_categories();
    sandbox.edit(|writer| {
        writer
            .library()
            .connection()
            .execute(
                "UPDATE djmdMyTag SET Name = 'Mood:Type' WHERE ID = ?1",
                [&mood],
            )
            .unwrap();
    });
    let dark = sandbox.create_my_tag_in(&mood, "Dark: very");
    sandbox.edit(|writer| {
        writer
            .set_my_tags(&fixture::track_id(7), std::slice::from_ref(&dark))
            .unwrap();
    });

    let found = page(&sandbox, &["--mytag-in", "Mood:Type", "Dark: very"]);
    assert_eq!(ids_of(&found), ["10007"]);
    assert_eq!(found.get("dropped"), None);
}

#[test]
fn a_key_is_every_spelling_of_it_and_a_name_off_the_wheel_is_exact() {
    let Catalogue { sandbox, .. } = catalogue();
    sandbox.describe_track(20, "Spelled", "x", "Am", 0);
    sandbox.describe_track(21, "Open Key", "x", "1m", 0);
    sandbox.describe_track(22, "Odd", "x", "Mystery", 0);

    let eight_a = ["10003", "10012", "10020", "10021"];
    assert_eq!(ids(&sandbox, &["--key", "8A"]), eight_a);
    assert_eq!(ids(&sandbox, &["--key", "Am"]), eight_a);
    assert_eq!(ids(&sandbox, &["--key", "Mystery"]), ["10022"]);
    assert_eq!(ids(&sandbox, &["--key", "mystery"]), Vec::<String>::new());
}

#[test]
fn energy_values_and_ranges_add_up_and_zero_is_no_energy() {
    let Catalogue { sandbox, .. } = catalogue();

    // 1 to 5 is every track with an Energy, not every track.
    assert_eq!(ids(&sandbox, &["--energy", "1..5"]), ["10003", "10012"]);
    assert_eq!(
        ids(
            &sandbox,
            &["--artist", "lovelace", "--energy", "0", "--energy", "5"]
        ),
        ["10007"]
    );
    assert_eq!(
        ids(
            &sandbox,
            &["--artist", "lovelace", "--energy", "0", "--energy", "4"]
        ),
        ["10003", "10007"]
    );
}

#[test]
fn bpm_ranges_in_hundredths_add_up() {
    let Catalogue { sandbox, .. } = catalogue();

    assert_eq!(
        ids(
            &sandbox,
            &[
                "--bpm-range",
                "128.10..128.12",
                "--bpm-range",
                "128.58..128.59"
            ]
        ),
        ["10010", "10011", "10012", "10058", "10059"]
    );
    // The window's whole-BPM bucket 129: 128.50 to 129.49.
    assert_eq!(
        ids(&sandbox, &["--bpm-range", "128.50..129.49"]),
        (10_050..10_060)
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
    );
    // An empty range admits nothing.
    assert_eq!(
        ids(&sandbox, &["--bpm-range", "0.02..0.01"]),
        Vec::<String>::new()
    );
    assert_eq!(refusal(&sandbox, &["--bpm-range", "fast"]), "bad_request");
}

#[test]
fn the_wire_takes_the_same_fields_the_window_sends() {
    let Catalogue { sandbox, .. } = catalogue();
    // The CLI starts the service; a second client then speaks to it.
    assert!(sandbox.cli(&["status"]).status.success());
    let mut client = sandbox.client();

    let page: Value = client
        .call(
            "tracks.query",
            json!({
                "keys": ["8A", "11B"],
                "prep_colours": ["Green", "Red"],
                "my_tag_picks": [{ "category": "Situation", "tags": ["Peak", "Warm-up"], "any": true }],
                "energies": ["4..5"],
                "bpm_ranges": ["127.50..128.49"],
            }),
        )
        .unwrap();

    let found: Vec<&str> = page["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].as_str().unwrap())
        .collect();
    assert_eq!(found, ["10003"]);
}
