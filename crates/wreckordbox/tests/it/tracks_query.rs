//! `wreckordbox tracks query` against a Library service on a throwaway
//! Library: the real binary, its output and its exit status.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use common::catalogue::{catalogue, Catalogue};
use common::Sandbox;
use serde_json::{json, Value};

/// The page `tracks query --json <args>` answers, which must succeed.
fn page(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json", "tracks", "query"];
    all.extend_from_slice(args);
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

/// The identifiers `tracks query <args>` matches, every page of them.
fn ids(sandbox: &Sandbox, args: &[&str]) -> Vec<String> {
    let mut all = args.to_vec();
    all.extend_from_slice(&["--limit", "1000"]);
    let page = page(sandbox, &all);
    assert_eq!(page["next_offset"], json!(null));
    ids_of(&page)
}

/// How many tracks `tracks query <args>` matches.
fn total(sandbox: &Sandbox, args: &[&str]) -> u64 {
    page(sandbox, args)["total"].as_u64().unwrap()
}

/// The error code `tracks query --json <args>` fails with.
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
fn a_query_with_no_filter_is_the_first_page_of_the_whole_collection() {
    let Catalogue { sandbox, fresh, .. } = catalogue();

    let first = page(&sandbox, &[]);

    // Sixty fixture tracks and the imported one; fifty to a page.
    assert_eq!(first["total"], 61);
    assert_eq!(first["offset"], 0);
    assert_eq!(first["limit"], 50);
    assert_eq!(first["next_offset"], 50);
    let expected: Vec<String> = (10_000..10_050).map(|id| id.to_string()).collect();
    assert_eq!(ids_of(&first), expected);

    let rest = page(&sandbox, &["--offset", "50"]);
    assert_eq!(rest["total"], 61);
    assert_eq!(rest["offset"], 50);
    assert_eq!(rest["next_offset"], json!(null));
    let mut expected: Vec<String> = (10_050..10_060).map(|id| id.to_string()).collect();
    expected.push(fresh);
    assert_eq!(ids_of(&rest), expected);
}

#[test]
fn each_row_is_the_tracks_full_record_with_energy_and_prep_colour_named() {
    let Catalogue { sandbox, .. } = catalogue();

    let found = page(&sandbox, &["--title", "midnight drive"]);

    let shown = sandbox.cli(&["--json", "tracks", "show", "10003"]).json();
    assert_eq!(found["tracks"], json!([shown]));
    let track = &found["tracks"][0];
    assert_eq!(track["energy"], 4);
    assert_eq!(track["rating"], 4);
    assert_eq!(track["prep_colour"], "Green");
    assert_eq!(track["colour"], "Green");
    assert_eq!(track["colour_id"], 5);
}

#[test]
fn a_limit_and_an_offset_page_through_the_matches() {
    let Catalogue { sandbox, .. } = catalogue();
    let genre = ["--genre", "house", "--limit", "1"];

    let first = page(&sandbox, &genre);
    assert_eq!(ids_of(&first), ["10003"]);
    assert_eq!(first["total"], 2);
    assert_eq!(first["limit"], 1);
    assert_eq!(first["next_offset"], 1);

    let second = page(&sandbox, &[&genre[..], &["--offset", "1"]].concat());
    assert_eq!(ids_of(&second), ["10007"]);
    assert_eq!(second["offset"], 1);
    assert_eq!(second["next_offset"], json!(null));

    // Past the end is an empty page, not a failure.
    let past = page(&sandbox, &[&genre[..], &["--offset", "5"]].concat());
    assert_eq!(ids_of(&past), Vec::<String>::new());
    assert_eq!(past["total"], 2);
    assert_eq!(past["next_offset"], json!(null));
}

#[test]
fn the_table_has_a_row_a_track_and_says_how_to_fetch_the_next_page() {
    let Catalogue { sandbox, .. } = catalogue();

    let run = sandbox.cli(&["tracks", "query", "--key", "8A", "--limit", "1"]);

    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{}", run.stdout);
    let cells = |line: &str| -> Vec<String> {
        line.split("  ")
            .map(str::trim)
            .filter(|cell| !cell.is_empty())
            .map(str::to_owned)
            .collect()
    };
    assert_eq!(
        cells(lines[0]),
        [
            "ID",
            "TITLE",
            "ARTIST",
            "GENRE",
            "BPM",
            "KEY",
            "ENERGY",
            "PREP COLOUR",
            "TIME"
        ]
    );
    assert_eq!(
        cells(lines[1]),
        [
            "10003",
            "Midnight Drive",
            "Ada Lovelace",
            "Tech House",
            "128.03",
            "8A",
            "4",
            "Green",
            "5:00"
        ]
    );
    assert_eq!(lines[2], "Tracks 1 to 1 of 2; next page: --offset 1");

    let last = sandbox.cli(&[
        "tracks", "query", "--key", "8A", "--limit", "1", "--offset", "1",
    ]);
    let lines: Vec<&str> = last.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{}", last.stdout);
    // No Prep colour is a dash, so the columns still line up.
    assert_eq!(
        cells(lines[1]),
        [
            "10012",
            "Midnight Oil",
            "Grace Hopper",
            "Techno",
            "128.12",
            "8A",
            "5",
            "-",
            "5:00"
        ]
    );
    assert_eq!(lines[2], "Tracks 2 to 2 of 2.");

    // Everything on one page needs no footer; nothing at all says so.
    let whole = sandbox.cli(&["tracks", "query", "--key", "8A"]);
    assert_eq!(whole.stdout.lines().count(), 3, "{}", whole.stdout);
    let none = sandbox.cli(&["tracks", "query", "--key", "1A"]);
    assert!(none.status.success(), "{}", none.stderr);
    assert_eq!(none.stdout, "No tracks.\n");
}

#[test]
fn text_fields_filter_on_what_they_contain_in_any_case() {
    let Catalogue { sandbox, .. } = catalogue();

    assert_eq!(ids(&sandbox, &["--title", "MIDNIGHT"]), ["10003", "10012"]);
    assert_eq!(ids(&sandbox, &["--artist", "hopper"]), ["10012"]);
    assert_eq!(ids(&sandbox, &["--album", "night"]), ["10003"]);
    assert_eq!(ids(&sandbox, &["--genre", "house"]), ["10003", "10007"]);
    assert_eq!(ids(&sandbox, &["--label", "compiler"]), ["10012"]);
    assert_eq!(ids(&sandbox, &["--comment", "intro"]), ["10003"]);
}

#[test]
fn the_search_text_matches_as_tracks_list_and_the_windows_search_box_do() {
    let Catalogue { sandbox, .. } = catalogue();

    for text in ["midnight", "lovelace", "midnight hopper", "nothing like it"] {
        let listed: Vec<String> = sandbox
            .cli(&["--json", "tracks", "list", "--filter", text])
            .json()["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|track| track["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids(&sandbox, &["--search", text]), listed, "{text}");
    }
    assert_eq!(ids(&sandbox, &["--search", "midnight hopper"]), ["10012"]);
}

#[test]
fn year_and_play_count_filter_on_a_value_or_a_range() {
    let Catalogue { sandbox, .. } = catalogue();

    assert_eq!(ids(&sandbox, &["--year", "2021"]), ["10003"]);
    assert_eq!(ids(&sandbox, &["--year", "2019..2021"]), ["10003", "10007"]);
    assert_eq!(ids(&sandbox, &["--year", "2022.."]), ["10012"]);
    // A track with no year is not before any year.
    assert_eq!(ids(&sandbox, &["--year", "..2019"]), ["10007"]);

    assert_eq!(ids(&sandbox, &["--play-count", "7"]), ["10003"]);
    assert_eq!(ids(&sandbox, &["--play-count", "1..5"]), ["10012"]);
    assert_eq!(ids(&sandbox, &["--play-count", "1.."]), ["10003", "10012"]);
}

#[test]
fn tempo_filters_on_a_whole_bpm_an_exact_one_or_a_range() {
    let Catalogue { sandbox, .. } = catalogue();

    // 128.50 to 128.59 are listed under 129, as the Track Filter lists them.
    let under_129: Vec<String> = (10_050..10_060).map(|id| id.to_string()).collect();
    assert_eq!(ids(&sandbox, &["--bpm", "129"]), under_129);
    assert_eq!(total(&sandbox, &["--bpm", "128"]), 50);
    assert_eq!(ids(&sandbox, &["--bpm", "128.03"]), ["10003"]);
    assert_eq!(
        ids(&sandbox, &["--bpm", "128.10..128.12"]),
        ["10010", "10011", "10012"]
    );
    assert_eq!(ids(&sandbox, &["--bpm", "128.58.."]), ["10058", "10059"]);
    // A track with no tempo is not slower than any tempo.
    assert_eq!(ids(&sandbox, &["--bpm", "..128.00"]), ["10000"]);
}

#[test]
fn key_energy_and_prep_colour_filter_as_the_track_filter_does() {
    let Catalogue { sandbox, .. } = catalogue();

    assert_eq!(ids(&sandbox, &["--key", "8A"]), ["10003", "10012"]);
    assert_eq!(ids(&sandbox, &["--key", "11B"]), ["10007"]);

    assert_eq!(ids(&sandbox, &["--energy", "4"]), ["10003"]);
    assert_eq!(ids(&sandbox, &["--energy", "4..5"]), ["10003", "10012"]);
    assert_eq!(ids(&sandbox, &["--energy", "1.."]), ["10003", "10012"]);
    // A track with no Energy is not below any Energy.
    assert_eq!(ids(&sandbox, &["--energy", "..4"]), ["10003"]);
    // The stars are where Energy is stored, so the option answers to both.
    assert_eq!(ids(&sandbox, &["--rating", "5"]), ["10012"]);
    assert_eq!(total(&sandbox, &["--energy", "0"]), 59);

    assert_eq!(ids(&sandbox, &["--prep-colour", "green"]), ["10003"]);
    assert_eq!(ids(&sandbox, &["--colour", "Red"]), ["10007"]);
    assert_eq!(total(&sandbox, &["--prep-colour", "none"]), 59);
}

#[test]
fn duration_date_added_location_and_analysis_state_filter() {
    let Catalogue { sandbox, fresh, .. } = catalogue();
    let fresh = [fresh];

    assert_eq!(ids(&sandbox, &["--duration", "95"]), fresh);
    assert_eq!(ids(&sandbox, &["--duration", "1:00..2:00"]), fresh);
    assert_eq!(total(&sandbox, &["--duration", "5:00"]), 60);
    assert_eq!(total(&sandbox, &["--duration", "2:00.."]), 60);

    // Only the imported track says when it was added: today.
    assert_eq!(ids(&sandbox, &["--date-added", "2020-01-01.."]), fresh);
    assert_eq!(total(&sandbox, &["--date-added", "..2020-01-01"]), 0);
    let today = sandbox.cli(&["--json", "tracks", "show", &fresh[0]]).json()["date_added"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(ids(&sandbox, &["--date-added", &today]), fresh);

    assert_eq!(ids(&sandbox, &["--location", "AUDIO/fresh cut"]), fresh);
    assert_eq!(ids(&sandbox, &["--location", "track007"]), ["10007"]);

    assert_eq!(ids(&sandbox, &["--analysed", "no"]), fresh);
    assert_eq!(total(&sandbox, &["--analysed", "yes"]), 60);
}

#[test]
fn a_mytag_filters_to_the_tracks_carrying_it_and_several_to_those_with_all() {
    let Catalogue { sandbox, .. } = catalogue();

    assert_eq!(ids(&sandbox, &["--mytag", "Peak"]), ["10003", "10012"]);
    assert_eq!(ids(&sandbox, &["--mytag", "Warm-up"]), ["10007"]);
    assert_eq!(
        ids(&sandbox, &["--mytag", "Peak", "--mytag", "Vocal"]),
        ["10012"]
    );
    assert_eq!(
        total(&sandbox, &["--mytag", "Peak", "--mytag", "Warm-up"]),
        0
    );

    assert_eq!(refusal(&sandbox, &["--mytag", "Sunset"]), "mytag_not_found");
}

#[test]
fn a_playlist_filters_to_its_tracks_in_its_own_order() {
    let Catalogue {
        sandbox,
        warm_up_set,
        ..
    } = catalogue();

    // By name and by identifier, in the order the DJ put them.
    assert_eq!(
        ids(&sandbox, &["--playlist", "Warm up set"]),
        ["10012", "10003"]
    );
    assert_eq!(
        ids(&sandbox, &["--playlist", &warm_up_set]),
        ["10012", "10003"]
    );
    assert_eq!(
        ids(&sandbox, &["--playlist", "900001"]),
        ["10005", "10006", "10007", "10008", "10009"]
    );
    // An intelligent playlist is whatever its rule admits.
    assert_eq!(ids(&sandbox, &["--playlist", "House"]), ["10003", "10007"]);

    assert_eq!(
        refusal(&sandbox, &["--playlist", "No such list"]),
        "playlist_not_found"
    );
}

#[test]
fn filters_combine_and_every_one_must_hold() {
    let Catalogue { sandbox, .. } = catalogue();

    assert_eq!(ids(&sandbox, &["--artist", "lovelace"]), ["10003", "10007"]);
    assert_eq!(
        ids(&sandbox, &["--artist", "lovelace", "--genre", "tech"]),
        ["10003"]
    );
    assert_eq!(
        ids(
            &sandbox,
            &["--key", "8A", "--energy", "5", "--mytag", "Peak"]
        ),
        ["10012"]
    );
    assert_eq!(
        ids(&sandbox, &["--playlist", "Playlist 0", "--key", "8A"]),
        ["10003"]
    );
    assert_eq!(
        ids(
            &sandbox,
            &[
                "--search",
                "midnight",
                "--year",
                "2020..",
                "--prep-colour",
                "none"
            ]
        ),
        ["10012"]
    );
    assert_eq!(
        ids(
            &sandbox,
            &[
                "--playlist",
                "House",
                "--bpm",
                "128",
                "--analysed",
                "yes",
                "--location",
                "track003"
            ]
        ),
        ["10003"]
    );
    assert_eq!(
        total(&sandbox, &["--artist", "lovelace", "--energy", "5"]),
        0
    );
}

#[test]
fn a_value_that_is_not_one_is_refused_and_nothing_is_listed() {
    let Catalogue { sandbox, .. } = catalogue();

    for args in [
        ["--energy", "9"],
        ["--energy", "high"],
        ["--year", "last"],
        ["--bpm", "fast"],
        ["--duration", "long"],
        ["--date-added", "yesterday"],
        ["--date-added", "2026-13-01"],
        ["--prep-colour", "Beige"],
    ] {
        assert_eq!(refusal(&sandbox, &args), "bad_request", "{args:?}");
    }

    let by_hand = sandbox.cli(&["tracks", "query", "--prep-colour", "Beige"]);
    assert!(!by_hand.status.success());
    assert_eq!(by_hand.stdout, "");
    assert!(by_hand.stderr.contains("Beige"), "{}", by_hand.stderr);
}

#[test]
fn a_client_that_names_no_arguments_gets_the_default_page() {
    let Catalogue { sandbox, .. } = catalogue();
    sandbox.status();
    let mut client = sandbox.client();

    let page: Value = client.call("tracks.query", json!({})).unwrap();
    assert_eq!(page["tracks"].as_array().unwrap().len(), 50);
    assert_eq!(page["total"], 61);
    assert_eq!(page["next_offset"], 50);

    // A misspelt argument is refused, not ignored into matching everything.
    let error = client
        .call::<Value>("tracks.query", json!({ "titel": "midnight" }))
        .unwrap_err();
    assert_eq!(error.code, "bad_request");
}
