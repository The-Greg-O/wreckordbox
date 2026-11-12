//! Reading the DJ's vocabulary and what depends on it (`mytags`,
//! `playlists`, `history`) against a Library service on a throwaway Library:
//! the real binary, its output and its exit status.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use common::Sandbox;
use rbl_db::fixture::{
    history_id, playlist_id, track_id, MY_TAG_CATEGORY, MY_TAG_PEAK, MY_TAG_WARM_UP,
};
use rbl_db::write::TrackField;
use serde_json::{json, Value};

/// What [`prepared`] made, by the identifiers the Library gave them.
struct Made {
    sandbox: Sandbox,
    /// A third tag, "Closer", made after the fixture's two and then moved
    /// between them.
    closer: String,
    /// The folder "Gigs" at the top of the playlist tree.
    gigs: String,
    /// The playlist "Friday" in "Gigs": tracks 5, 2 and 9, in that order.
    friday: String,
    /// The intelligent playlist "Peak by Ada" in "Gigs".
    peak_by_ada: String,
}

/// The fixture Library (40 tracks, three playlists of five, two history
/// sessions of five, one MyTag category "Situation" holding "Peak" and
/// "Warm-up") given a vocabulary in use, all through the Library's own
/// writer:
///
/// - the category's tags reordered to Warm-up, Closer, Peak;
/// - track 1 tagged Peak, track 2 Peak and Warm-up, track 3 Warm-up;
/// - track 2 by "Ada Lovelace";
/// - a folder holding a playlist and an intelligent playlist whose rule is
///   "tagged Peak and artist contains Ada";
/// - track 0 played a second time at the end of the first history session.
fn prepared() -> Made {
    let sandbox = Sandbox::new();
    let mut closer = String::new();
    let mut gigs = String::new();
    let mut friday = String::new();
    let mut peak_by_ada = String::new();
    sandbox.edit(|writer| {
        closer = writer.create_my_tag(MY_TAG_CATEGORY).unwrap().id;
        writer.rename_my_tag(&closer, "Closer").unwrap();
        writer
            .reorder_my_tags_with_undo(&[
                MY_TAG_WARM_UP.to_owned(),
                closer.clone(),
                MY_TAG_PEAK.to_owned(),
            ])
            .unwrap();
        writer
            .set_my_tags(&track_id(1), &[MY_TAG_PEAK.to_owned()])
            .unwrap();
        writer
            .set_my_tags(
                &track_id(2),
                &[MY_TAG_PEAK.to_owned(), MY_TAG_WARM_UP.to_owned()],
            )
            .unwrap();
        writer
            .set_my_tags(&track_id(3), &[MY_TAG_WARM_UP.to_owned()])
            .unwrap();
        writer
            .set_field(&track_id(2), TrackField::Artist, "Ada Lovelace")
            .unwrap();

        gigs = writer.create_folder("Gigs", "root").unwrap();
        friday = writer.create_playlist("Friday", &gigs).unwrap();
        writer
            .add_tracks(&friday, &[track_id(5), track_id(2), track_id(9)])
            .unwrap();
        peak_by_ada = writer
            .create_smart_playlist("Peak by Ada", &gigs, |id| {
                format!(
                    r#"<NODE Id="{id}" LogicalOperator="1" AutomaticUpdate="1"><CONDITION PropertyName="myTag" Operator="8" ValueUnit="" ValueLeft="{MY_TAG_PEAK}" ValueRight=""/><CONDITION PropertyName="artist" Operator="8" ValueUnit="" ValueLeft="Ada" ValueRight=""/></NODE>"#
                )
            })
            .unwrap();

        writer.add_to_history(&history_id(0), &track_id(0)).unwrap();
    });
    Made {
        sandbox,
        closer,
        gigs,
        friday,
        peak_by_ada,
    }
}

/// A successful run's JSON.
fn json_of(sandbox: &Sandbox, args: &[&str]) -> Value {
    let run = sandbox.cli(args);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    run.json()
}

/// A successful run's lines, each split into its cells: columns are two or
/// more spaces apart, so a name with a space in it stays one cell.
fn table_of(sandbox: &Sandbox, args: &[&str]) -> Vec<Vec<String>> {
    let run = sandbox.cli(args);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    run.stdout
        .lines()
        .map(|line| {
            line.split("  ")
                .map(str::trim)
                .filter(|cell| !cell.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .collect()
}

/// The `id` of every entry of a JSON array.
fn ids(list: &Value) -> Vec<&str> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect()
}

#[test]
fn mytags_list_gives_the_categories_and_their_tags_in_stored_order_with_track_counts() {
    let made = prepared();

    let result = json_of(&made.sandbox, &["--json", "mytags", "list"]);

    assert_eq!(
        result,
        json!({
            "categories": [{
                "id": MY_TAG_CATEGORY,
                "name": "Situation",
                "tags": [
                    { "id": MY_TAG_WARM_UP, "name": "Warm-up", "tracks": 2 },
                    { "id": made.closer, "name": "Closer", "tracks": 0 },
                    { "id": MY_TAG_PEAK, "name": "Peak", "tracks": 2 },
                ],
            }],
        })
    );
}

#[test]
fn mytags_list_prints_a_table_of_the_tags_under_their_category() {
    let made = prepared();

    let table = table_of(&made.sandbox, &["mytags", "list"]);

    assert_eq!(
        table,
        [
            vec!["CATEGORY", "CATEGORY ID", "TAG", "TAG ID", "TRACKS"],
            vec!["Situation", MY_TAG_CATEGORY, "Warm-up", MY_TAG_WARM_UP, "2"],
            vec!["Situation", MY_TAG_CATEGORY, "Closer", &made.closer, "0"],
            vec!["Situation", MY_TAG_CATEGORY, "Peak", MY_TAG_PEAK, "2"],
        ]
    );
}

#[test]
fn mytags_show_gives_the_tags_a_track_carries_in_the_catalogue_order() {
    let made = prepared();

    // Track 2 was tagged Peak first, then Warm-up; Warm-up is listed first.
    let result = json_of(&made.sandbox, &["--json", "mytags", "show", &track_id(2)]);

    let situation = |id: &str, name: &str| json!({ "id": id, "name": name, "category_id": MY_TAG_CATEGORY, "category": "Situation" });
    assert_eq!(
        result,
        json!({
            "id": track_id(2),
            "title": "Track 002",
            "my_tags": [situation(MY_TAG_WARM_UP, "Warm-up"), situation(MY_TAG_PEAK, "Peak")],
        })
    );

    let table = table_of(&made.sandbox, &["mytags", "show", &track_id(2)]);
    assert_eq!(
        table,
        [
            vec!["Track 10002 \"Track 002\""],
            vec!["CATEGORY", "TAG", "TAG ID"],
            vec!["Situation", "Warm-up", MY_TAG_WARM_UP],
            vec!["Situation", "Peak", MY_TAG_PEAK],
        ]
    );
}

#[test]
fn mytags_show_says_so_when_a_track_has_no_tags_and_fails_for_no_such_track() {
    let made = prepared();

    let untagged = json_of(&made.sandbox, &["--json", "mytags", "show", &track_id(7)]);
    assert_eq!(untagged["my_tags"], json!([]));
    let text = made.sandbox.cli(&["mytags", "show", &track_id(7)]);
    assert_eq!(text.stdout, "Track 10007 \"Track 007\" has no MyTags.\n");

    let missing = made.sandbox.cli(&["--json", "mytags", "show", "424242"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(missing.stdout, "");
    assert_eq!(missing.error_json()["error"]["code"], "track_not_found");
}

#[test]
fn mytags_tracks_lists_the_tracks_carrying_a_tag() {
    let made = prepared();

    let result = json_of(&made.sandbox, &["--json", "mytags", "tracks", MY_TAG_PEAK]);

    assert_eq!(
        result["my_tag"],
        json!({ "id": MY_TAG_PEAK, "name": "Peak", "category_id": MY_TAG_CATEGORY, "category": "Situation" })
    );
    assert_eq!(result["total"], 2);
    assert_eq!(ids(&result["tracks"]), [track_id(1), track_id(2)]);
    assert_eq!(
        result["tracks"][1],
        json!({ "id": track_id(2), "title": "Track 002", "artist": "Ada Lovelace", "bpm": 128.02, "key": "", "rating": 0 })
    );

    let table = table_of(&made.sandbox, &["mytags", "tracks", MY_TAG_PEAK]);
    assert_eq!(
        table,
        [
            vec!["MyTag 700002 \"Peak\" in Situation: 2 tracks"],
            vec!["ID", "TITLE", "ARTIST", "BPM", "KEY", "RATING"],
            vec!["10001", "Track 001", "-", "128.01", "-", "0"],
            vec!["10002", "Track 002", "Ada Lovelace", "128.02", "-", "0"],
        ]
    );
}

#[test]
fn mytags_tracks_is_cut_to_the_limit_and_fails_for_no_such_tag() {
    let made = prepared();

    let limited = json_of(
        &made.sandbox,
        &["--json", "mytags", "tracks", MY_TAG_PEAK, "--limit", "1"],
    );
    assert_eq!(limited["total"], 2);
    assert_eq!(ids(&limited["tracks"]), [track_id(1)]);
    let text = made
        .sandbox
        .cli(&["mytags", "tracks", MY_TAG_PEAK, "--limit", "1"]);
    assert_eq!(
        text.stdout.lines().last(),
        Some("Showing 1 of 2 tracks; raise --limit to see more.")
    );

    // A tag nothing carries is still a tag.
    let unused = json_of(&made.sandbox, &["--json", "mytags", "tracks", &made.closer]);
    assert_eq!(unused["total"], 0);
    assert_eq!(unused["tracks"], json!([]));

    // A category is not a tag.
    let missing = made
        .sandbox
        .cli(&["--json", "mytags", "tracks", MY_TAG_CATEGORY]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(missing.error_json()["error"]["code"], "my_tag_not_found");
}

#[test]
fn playlists_tree_gives_folders_playlists_and_intelligent_playlists_with_their_rules() {
    let made = prepared();

    let result = json_of(&made.sandbox, &["--json", "playlists", "tree"]);

    let top = |index: usize| {
        json!({
            "id": playlist_id(index), "name": format!("Playlist {index}"), "kind": "playlist",
            "parent_id": null, "depth": 1, "tracks": 5,
        })
    };
    assert_eq!(
        result,
        json!({
            "playlists": [
                top(0),
                top(1),
                top(2),
                { "id": made.gigs, "name": "Gigs", "kind": "folder", "parent_id": null, "depth": 1, "tracks": null },
                { "id": made.friday, "name": "Friday", "kind": "playlist", "parent_id": made.gigs, "depth": 2, "tracks": 3 },
                {
                    "id": made.peak_by_ada, "name": "Peak by Ada", "kind": "intelligent",
                    "parent_id": made.gigs, "depth": 2, "tracks": 1,
                    "rule": {
                        "match": "all",
                        "conditions": [
                            {
                                "property": "my_tag", "operator": "contains", "value": MY_TAG_PEAK,
                                "my_tag": {
                                    "id": MY_TAG_PEAK, "name": "Peak",
                                    "category_id": MY_TAG_CATEGORY, "category": "Situation",
                                },
                            },
                            { "property": "artist", "operator": "contains", "value": "Ada" },
                        ],
                    },
                },
            ],
        })
    );
}

#[test]
fn playlists_tree_prints_the_tree_with_each_rule_on_its_line() {
    let made = prepared();

    let table = table_of(&made.sandbox, &["playlists", "tree"]);

    assert_eq!(
        table,
        [
            vec!["ID", "KIND", "TRACKS", "NAME", "RULE"],
            vec!["900000", "playlist", "5", "Playlist 0"],
            vec!["900001", "playlist", "5", "Playlist 1"],
            vec!["900002", "playlist", "5", "Playlist 2"],
            vec![&made.gigs, "folder", "-", "Gigs"],
            vec![&made.friday, "playlist", "3", "Friday"],
            vec![
                &made.peak_by_ada,
                "intelligent",
                "1",
                "Peak by Ada",
                "all of: my_tag contains Peak [700002]; artist contains Ada"
            ],
        ]
    );
    // What is in a folder is indented under it.
    let run = made.sandbox.cli(&["playlists", "tree"]);
    let gigs = run.stdout.lines().find(|l| l.contains("Gigs")).unwrap();
    let friday = run.stdout.lines().find(|l| l.contains("Friday")).unwrap();
    assert_eq!(friday.find("Friday"), gigs.find("Gigs").map(|at| at + 2));
}

#[test]
fn playlists_tracks_gives_a_playlists_tracks_in_its_order() {
    let made = prepared();

    let result = json_of(
        &made.sandbox,
        &["--json", "playlists", "tracks", &made.friday],
    );

    assert_eq!(
        result["playlist"],
        json!({ "id": made.friday, "name": "Friday", "kind": "playlist" })
    );
    assert_eq!(result["total"], 3);
    // The order they were put in, not the collection's.
    assert_eq!(
        ids(&result["tracks"]),
        [track_id(5), track_id(2), track_id(9)]
    );
    // Each is an entry of the playlist, with an identifier of its own that
    // is neither its position nor its track's.
    let entry = result["tracks"][1]["entry_id"].as_str().unwrap().to_owned();
    assert!(!entry.is_empty());
    assert_ne!(entry, track_id(2));
    assert_ne!(result["tracks"][0]["entry_id"], entry.as_str());
    assert_eq!(
        result["tracks"][1],
        json!({
            "position": 2, "entry_id": entry, "id": track_id(2), "title": "Track 002",
            "artist": "Ada Lovelace", "bpm": 128.02, "key": "", "rating": 0,
        })
    );

    let table = table_of(&made.sandbox, &["playlists", "tracks", &made.friday]);
    assert_eq!(
        table,
        [
            vec![format!("Playlist {} \"Friday\": 3 tracks", made.friday)],
            ["#", "ID", "TITLE", "ARTIST", "BPM", "KEY", "RATING"]
                .map(str::to_owned)
                .to_vec(),
            ["1", "10005", "Track 005", "-", "128.05", "-", "0"]
                .map(str::to_owned)
                .to_vec(),
            [
                "2",
                "10002",
                "Track 002",
                "Ada Lovelace",
                "128.02",
                "-",
                "0"
            ]
            .map(str::to_owned)
            .to_vec(),
            ["3", "10009", "Track 009", "-", "128.09", "-", "0"]
                .map(str::to_owned)
                .to_vec(),
        ]
    );
}

#[test]
fn playlists_tracks_lists_what_an_intelligent_playlists_rule_admits_and_a_folders_playlists() {
    let made = prepared();

    // Tracks 1 and 2 are tagged Peak; only track 2 is by Ada.
    let intelligent = json_of(
        &made.sandbox,
        &["--json", "playlists", "tracks", &made.peak_by_ada],
    );
    assert_eq!(intelligent["playlist"]["kind"], "intelligent");
    assert_eq!(ids(&intelligent["tracks"]), [track_id(2)]);

    // Friday's three, then what the rule admits that Friday did not list.
    let folder = json_of(
        &made.sandbox,
        &["--json", "playlists", "tracks", &made.gigs],
    );
    assert_eq!(folder["playlist"]["kind"], "folder");
    assert_eq!(folder["total"], 3);
    assert_eq!(
        ids(&folder["tracks"]),
        [track_id(5), track_id(2), track_id(9)]
    );
}

#[test]
fn playlists_tracks_is_cut_to_the_limit_and_fails_for_no_such_playlist() {
    let made = prepared();

    let limited = json_of(
        &made.sandbox,
        &[
            "--json",
            "playlists",
            "tracks",
            &playlist_id(1),
            "--limit",
            "2",
        ],
    );
    assert_eq!(limited["total"], 5);
    assert_eq!(ids(&limited["tracks"]), [track_id(5), track_id(6)]);
    let text = made
        .sandbox
        .cli(&["playlists", "tracks", &playlist_id(1), "--limit", "2"]);
    assert_eq!(
        text.stdout.lines().last(),
        Some("Showing 2 of 5 tracks; raise --limit to see more.")
    );

    let missing = made
        .sandbox
        .cli(&["--json", "playlists", "tracks", "424242"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(missing.stdout, "");
    assert_eq!(missing.error_json()["error"]["code"], "playlist_not_found");
}

#[test]
fn history_list_gives_the_occasions_oldest_first_with_where_they_are_filed() {
    let made = prepared();

    let result = json_of(&made.sandbox, &["--json", "history", "list"]);

    assert_eq!(
        result,
        json!({
            "occasions": [
                { "id": history_id(0), "name": "HISTORY 2026-09-01", "folder": "2026/September", "tracks": 6 },
                { "id": history_id(1), "name": "HISTORY 2026-09-02", "folder": "2026/September", "tracks": 5 },
            ],
        })
    );

    let table = table_of(&made.sandbox, &["history", "list"]);
    assert_eq!(
        table,
        [
            vec!["ID", "OCCASION", "FILED UNDER", "PLAYS"],
            vec!["800000", "HISTORY 2026-09-01", "2026/September", "6"],
            vec!["800001", "HISTORY 2026-09-02", "2026/September", "5"],
        ]
    );
}

#[test]
fn history_tracks_gives_an_occasions_plays_in_played_order_with_repeats() {
    let made = prepared();

    let result = json_of(
        &made.sandbox,
        &["--json", "history", "tracks", &history_id(0)],
    );

    assert_eq!(
        result["occasion"],
        json!({ "id": history_id(0), "name": "HISTORY 2026-09-01", "folder": "2026/September", "tracks": 6 })
    );
    assert_eq!(result["total"], 6);
    // Track 0 opened the night and was played again at the end.
    let played: Vec<String> = [0, 1, 2, 3, 4, 0].map(track_id).to_vec();
    assert_eq!(ids(&result["tracks"]), played);
    assert_eq!(result["tracks"][5]["position"], 6);

    let table = table_of(&made.sandbox, &["history", "tracks", &history_id(0)]);
    assert_eq!(table[0], ["History 800000 \"HISTORY 2026-09-01\": 6 plays"]);
    assert_eq!(
        table[1],
        ["#", "ID", "TITLE", "ARTIST", "BPM", "KEY", "RATING"]
    );
    assert_eq!(
        table[7],
        ["6", "10000", "Track 000", "-", "128.00", "-", "0"]
    );
}

#[test]
fn history_tracks_is_cut_to_the_limit_and_fails_for_what_is_not_an_occasion() {
    let made = prepared();

    let limited = json_of(
        &made.sandbox,
        &[
            "--json",
            "history",
            "tracks",
            &history_id(1),
            "--limit",
            "2",
        ],
    );
    assert_eq!(limited["total"], 5);
    assert_eq!(ids(&limited["tracks"]), [track_id(5), track_id(6)]);
    let text = made
        .sandbox
        .cli(&["history", "tracks", &history_id(1), "--limit", "2"]);
    assert_eq!(
        text.stdout.lines().last(),
        Some("Showing 2 of 5 plays; raise --limit to see more.")
    );

    // The month folder the occasions are filed under is not an occasion.
    for not_an_occasion in ["202609", "424242"] {
        let missing = made
            .sandbox
            .cli(&["--json", "history", "tracks", not_an_occasion]);
        assert_eq!(missing.status.code(), Some(1));
        assert_eq!(missing.error_json()["error"]["code"], "history_not_found");
    }
}

#[test]
fn an_empty_library_reads_as_nothing_and_not_as_an_error() {
    let sandbox = Sandbox::empty();

    // The fixture's one category and two tags are there with nothing tagged.
    let tags = json_of(&sandbox, &["--json", "mytags", "list"]);
    assert_eq!(tags["categories"][0]["tags"][0]["tracks"], 0);
    assert_eq!(
        json_of(&sandbox, &["--json", "playlists", "tree"]),
        json!({ "playlists": [] })
    );
    assert_eq!(
        json_of(&sandbox, &["--json", "history", "list"]),
        json!({ "occasions": [] })
    );
    assert_eq!(
        sandbox.cli(&["playlists", "tree"]).stdout,
        "No playlists.\n"
    );
    assert_eq!(
        sandbox.cli(&["history", "list"]).stdout,
        "No play history.\n"
    );
}
