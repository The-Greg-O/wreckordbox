//! Editing MyTags (`mytags create | rename | move | reorder | merge |
//! delete | assign | unassign`) against a Library service on a throwaway
//! Library with rekordbox's four MyTag categories: the real binary, what
//! the read commands then show, and what a second connected client is told.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use std::time::Duration;

use common::{Details, Sandbox};
use rbl_db::fixture::{track_id, MY_TAG_CATEGORY, MY_TAG_PEAK, MY_TAG_WARM_UP};
use serde_json::{json, Value};
use wreckordbox_service::Client;

/// A Library as a DJ's is laid out, by the identifiers it gave things.
///
/// | Category  | Tags (in order)            |
/// |-----------|----------------------------|
/// | Situation | Peak, Warm-up              |
/// | Mood      | Dark, Uplifting            |
/// | Sound     | Acid                       |
/// | Sub-genre | (none)                     |
///
/// Track 1 carries Peak and Dark; track 2 Peak, Warm-up and Acid; track 3
/// Dark.
struct Made {
    sandbox: Sandbox,
    mood: String,
    sound: String,
    sub_genre: String,
    dark: String,
    uplifting: String,
    acid: String,
}

fn prepared() -> Made {
    let sandbox = Sandbox::new();
    let [_, mood, sound, sub_genre] = sandbox.four_categories();
    let dark = sandbox.create_my_tag_in(&mood, "Dark");
    let uplifting = sandbox.create_my_tag_in(&mood, "Uplifting");
    let acid = sandbox.create_my_tag_in(&sound, "Acid");
    for (index, tags) in [
        (1, vec![MY_TAG_PEAK, dark.as_str()]),
        (2, vec![MY_TAG_PEAK, MY_TAG_WARM_UP, acid.as_str()]),
        (3, vec![dark.as_str()]),
    ] {
        sandbox.detail_track(
            index,
            &Details {
                my_tags: &tags,
                ..Details::default()
            },
        );
    }
    Made {
        sandbox,
        mood,
        sound,
        sub_genre,
        dark,
        uplifting,
        acid,
    }
}

/// A successful run's JSON.
fn json_of(sandbox: &Sandbox, args: &[&str]) -> Value {
    let run = sandbox.cli(args);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(run.stderr, "");
    run.json()
}

/// The code a refused run reports, having printed nothing.
fn refusal(sandbox: &Sandbox, args: &[&str]) -> String {
    let mut with_json = vec!["--json"];
    with_json.extend_from_slice(args);
    let run = sandbox.cli(&with_json);
    assert!(!run.status.success(), "{}", run.stdout);
    assert_eq!(common::without_run(&run.stdout), "");
    run.error_json()["error"]["code"]
        .as_str()
        .expect(&run.stderr)
        .to_owned()
}

/// `mytags list` as `(category, [(tag, tracks)])`, in order.
fn vocabulary(sandbox: &Sandbox) -> Vec<(String, Vec<(String, u64)>)> {
    json_of(sandbox, &["--json", "mytags", "list"])["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| {
            (
                category["name"].as_str().unwrap().to_owned(),
                category["tags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|tag| {
                        (
                            tag["name"].as_str().unwrap().to_owned(),
                            tag["tracks"].as_u64().unwrap(),
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

/// The vocabulary [`prepared`] makes.
fn as_prepared() -> Vec<(String, Vec<(String, u64)>)> {
    named(&[
        ("Situation", &[("Peak", 2), ("Warm-up", 1)]),
        ("Mood", &[("Dark", 2), ("Uplifting", 0)]),
        ("Sound", &[("Acid", 1)]),
        ("Sub-genre", &[]),
    ])
}

fn named(categories: &[(&str, &[(&str, u64)])]) -> Vec<(String, Vec<(String, u64)>)> {
    categories
        .iter()
        .map(|(category, tags)| {
            (
                (*category).to_owned(),
                tags.iter()
                    .map(|(tag, tracks)| ((*tag).to_owned(), *tracks))
                    .collect(),
            )
        })
        .collect()
}

/// The names of the MyTags a track carries, as `mytags show` gives them.
fn tags_of(sandbox: &Sandbox, track: usize) -> Vec<String> {
    json_of(sandbox, &["--json", "mytags", "show", &track_id(track)])["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| tag["name"].as_str().unwrap().to_owned())
        .collect()
}

/// A track's comment, as `tracks show` gives it.
fn comment_of(sandbox: &Sandbox, track: usize) -> String {
    json_of(sandbox, &["--json", "tracks", "show", &track_id(track)])["comment"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// A second client of the sandbox's service, listening for events.
fn watcher(sandbox: &Sandbox) -> Client {
    sandbox.status();
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    watcher
}

/// The one change event the last edit sent `watcher`, as its generation.
fn told_of_one_change(watcher: &mut Client) -> u64 {
    let event = watcher
        .next_event(Duration::from_secs(5))
        .unwrap()
        .expect("no event arrived");
    assert_eq!(event.event, "library_changed");
    assert_eq!(event.data["kind"], "library");
    let generation = event.data["generation"].as_u64().unwrap();
    assert!(
        watcher
            .next_event(Duration::from_millis(200))
            .unwrap()
            .is_none(),
        "one edit sent more than one event"
    );
    generation
}

fn told_nothing(watcher: &mut Client) {
    assert!(watcher
        .next_event(Duration::from_millis(300))
        .unwrap()
        .is_none());
}

#[test]
fn the_read_commands_cover_all_four_categories() {
    let made = prepared();
    let sandbox = &made.sandbox;

    assert_eq!(vocabulary(sandbox), as_prepared());
    let list = json_of(sandbox, &["--json", "mytags", "list"]);
    let category_ids: Vec<&str> = list["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|category| category["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        category_ids,
        [MY_TAG_CATEGORY, &made.mood, &made.sound, &made.sub_genre]
    );

    // A track's tags come in the catalogue's order, each with its category.
    let shown = json_of(sandbox, &["--json", "mytags", "show", &track_id(2)]);
    assert_eq!(
        shown["my_tags"],
        json!([
            { "id": MY_TAG_PEAK, "name": "Peak", "category_id": MY_TAG_CATEGORY, "category": "Situation" },
            { "id": MY_TAG_WARM_UP, "name": "Warm-up", "category_id": MY_TAG_CATEGORY, "category": "Situation" },
            { "id": made.acid, "name": "Acid", "category_id": made.sound, "category": "Sound" },
        ])
    );

    // The tracks of a tag outside the first category.
    let dark = json_of(sandbox, &["--json", "mytags", "tracks", &made.dark]);
    assert_eq!(dark["my_tag"]["category"], "Mood");
    assert_eq!(dark["total"], 2);
    let carriers: Vec<&str> = dark["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].as_str().unwrap())
        .collect();
    assert_eq!(carriers, [track_id(1), track_id(3)]);

    // A category is not a tag, whichever category it is.
    assert_eq!(
        refusal(sandbox, &["mytags", "tracks", &made.sub_genre]),
        "my_tag_not_found"
    );

    // By hand, an empty category still has its line.
    let text = sandbox.cli(&["mytags", "list"]).stdout;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 7, "{text}");
    assert!(lines[1].starts_with("Situation"), "{text}");
    assert!(lines[3].starts_with("Mood"), "{text}");
    assert!(lines[5].starts_with("Sound"), "{text}");
    assert!(lines[6].starts_with("Sub-genre"), "{text}");
}

#[test]
fn creating_a_tag_puts_it_at_the_end_of_its_category_and_tells_a_second_client() {
    let made = prepared();
    let sandbox = &made.sandbox;
    let mut watcher = watcher(sandbox);

    let created = json_of(
        sandbox,
        &["--json", "mytags", "create", &made.mood, "Melancholic"],
    );

    assert_eq!(created["my_tag"]["name"], "Melancholic");
    assert_eq!(created["my_tag"]["category"], "Mood");
    assert_eq!(created["my_tag"]["category_id"], made.mood.as_str());
    assert_eq!(created["generation"], told_of_one_change(&mut watcher));
    assert_eq!(
        vocabulary(sandbox),
        named(&[
            ("Situation", &[("Peak", 2), ("Warm-up", 1)]),
            ("Mood", &[("Dark", 2), ("Uplifting", 0), ("Melancholic", 0)]),
            ("Sound", &[("Acid", 1)]),
            ("Sub-genre", &[]),
        ])
    );
    // The identifier it answered is the one the Library now lists.
    let id = created["my_tag"]["id"].as_str().unwrap();
    let tracks = json_of(sandbox, &["--json", "mytags", "tracks", id]);
    assert_eq!(tracks["total"], 0);

    // By hand, one line.
    let run = sandbox.cli(&["mytags", "create", &made.sub_genre, "Tech House"]);
    assert!(run.status.success(), "{}", run.stderr);
    let made_id = vocabulary_ids(sandbox, "Sub-genre")[0].clone();
    assert_eq!(
        common::without_run(&run.stdout),
        format!("Created MyTag {made_id} \"Tech House\" in Sub-genre.\n")
    );
}

/// The identifiers of a category's tags, in order.
fn vocabulary_ids(sandbox: &Sandbox, category: &str) -> Vec<String> {
    json_of(sandbox, &["--json", "mytags", "list"])["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == category)
        .unwrap()["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| tag["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn renaming_a_tag_keeps_its_tracks_and_a_category_can_be_renamed_too() {
    let made = prepared();
    let sandbox = &made.sandbox;
    let mut watcher = watcher(sandbox);

    let renamed = json_of(
        sandbox,
        &["--json", "mytags", "rename", &made.dark, "Moody"],
    );

    assert_eq!(renamed["id"], made.dark.as_str());
    assert_eq!(renamed["name"], "Moody");
    assert_eq!(renamed["previous_name"], "Dark");
    assert_eq!(renamed["kind"], "tag");
    assert_eq!(renamed["generation"], told_of_one_change(&mut watcher));
    assert_eq!(tags_of(sandbox, 1), ["Peak", "Moody"]);

    let run = sandbox.cli(&["mytags", "rename", &made.mood, "Mood/Emotion"]);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(
        common::without_run(&run.stdout),
        format!(
            "Renamed MyTag category {} from \"Mood\" to \"Mood/Emotion\".\n",
            made.mood
        )
    );
    told_of_one_change(&mut watcher);
    assert_eq!(
        vocabulary(sandbox),
        named(&[
            ("Situation", &[("Peak", 2), ("Warm-up", 1)]),
            ("Mood/Emotion", &[("Moody", 2), ("Uplifting", 0)]),
            ("Sound", &[("Acid", 1)]),
            ("Sub-genre", &[]),
        ])
    );
}

#[test]
fn moving_a_tag_gives_it_a_new_place_in_its_category() {
    let made = prepared();
    let sandbox = &made.sandbox;
    let melancholic = json_of(
        sandbox,
        &["--json", "mytags", "create", &made.mood, "Melancholic"],
    )["my_tag"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut watcher = watcher(sandbox);

    // Dark, Uplifting, Melancholic: the last goes first.
    let moved = json_of(sandbox, &["--json", "mytags", "move", &melancholic, "1"]);

    assert_eq!(
        moved["order"],
        json!([
            { "id": melancholic, "name": "Melancholic" },
            { "id": made.dark, "name": "Dark" },
            { "id": made.uplifting, "name": "Uplifting" },
        ])
    );
    assert_eq!(moved["category"], "Mood");
    assert_eq!(moved["generation"], told_of_one_change(&mut watcher));
    assert_eq!(
        vocabulary_ids(sandbox, "Mood"),
        [
            melancholic.clone(),
            made.dark.clone(),
            made.uplifting.clone()
        ]
    );

    // A place past the end is the end.
    let run = sandbox.cli(&["mytags", "move", &melancholic, "9"]);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(
        common::without_run(&run.stdout),
        "Mood is now in the order: Dark, Uplifting, Melancholic.\n"
    );
    assert_eq!(
        vocabulary_ids(sandbox, "Mood"),
        [made.dark.clone(), made.uplifting.clone(), melancholic]
    );
    // The other categories are as they were.
    assert_eq!(
        vocabulary_ids(sandbox, "Situation"),
        [MY_TAG_PEAK, MY_TAG_WARM_UP]
    );
}

#[test]
fn reordering_names_every_tag_of_a_category_or_every_category() {
    let made = prepared();
    let sandbox = &made.sandbox;
    let mut watcher = watcher(sandbox);

    let reordered = json_of(
        sandbox,
        &["--json", "mytags", "reorder", MY_TAG_WARM_UP, MY_TAG_PEAK],
    );
    assert_eq!(
        reordered["order"],
        json!([
            { "id": MY_TAG_WARM_UP, "name": "Warm-up" },
            { "id": MY_TAG_PEAK, "name": "Peak" },
        ])
    );
    assert_eq!(reordered["category"], "Situation");
    assert_eq!(reordered["generation"], told_of_one_change(&mut watcher));

    // The categories themselves: the order the deck's Track Filter shows.
    let categories = json_of(
        sandbox,
        &[
            "--json",
            "mytags",
            "reorder",
            &made.sub_genre,
            &made.mood,
            &made.sound,
            MY_TAG_CATEGORY,
        ],
    );
    assert_eq!(categories["category"], Value::Null);
    told_of_one_change(&mut watcher);
    assert_eq!(
        vocabulary(sandbox),
        named(&[
            ("Sub-genre", &[]),
            ("Mood", &[("Dark", 2), ("Uplifting", 0)]),
            ("Sound", &[("Acid", 1)]),
            ("Situation", &[("Warm-up", 1), ("Peak", 2)]),
        ])
    );

    // An order that leaves a tag out, repeats one or mixes categories is
    // refused whole.
    for order in [
        vec![MY_TAG_PEAK],
        vec![MY_TAG_PEAK, MY_TAG_PEAK, MY_TAG_WARM_UP],
        vec![MY_TAG_PEAK, MY_TAG_WARM_UP, made.dark.as_str()],
    ] {
        let mut args = vec!["mytags", "reorder"];
        args.extend(order);
        assert_eq!(refusal(sandbox, &args), "invalid_my_tag_order");
    }
    assert_eq!(
        refusal(sandbox, &["mytags", "reorder", MY_TAG_PEAK, "12345"]),
        "my_tag_not_found"
    );
    told_nothing(&mut watcher);
    assert_eq!(
        vocabulary_ids(sandbox, "Situation"),
        [MY_TAG_WARM_UP, MY_TAG_PEAK]
    );
}

#[test]
fn assigning_puts_a_tag_on_many_tracks_and_unassigning_takes_it_off() {
    let made = prepared();
    let sandbox = &made.sandbox;
    let mut watcher = watcher(sandbox);

    // Track 3 already carries Dark; 4 and 5 do not.
    let assigned = json_of(
        sandbox,
        &[
            "--json",
            "mytags",
            "assign",
            &made.dark,
            &track_id(3),
            &track_id(4),
            &track_id(5),
        ],
    );

    assert_eq!(assigned["my_tag"]["name"], "Dark");
    assert_eq!(assigned["changed"], 2);
    assert_eq!(
        assigned["tracks"],
        json!([
            { "id": track_id(3), "title": "Track 003", "changed": false },
            { "id": track_id(4), "title": "Track 004", "changed": true },
            { "id": track_id(5), "title": "Track 005", "changed": true },
        ])
    );
    assert_eq!(assigned["generation"], told_of_one_change(&mut watcher));
    assert_eq!(tags_of(sandbox, 4), ["Dark"]);
    let carriers = json_of(sandbox, &["--json", "mytags", "tracks", &made.dark]);
    assert_eq!(carriers["total"], 4);

    // One track, by hand; its other tags stay.
    let run = sandbox.cli(&["mytags", "unassign", MY_TAG_PEAK, &track_id(2)]);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(
        common::without_run(&run.stdout),
        "Took MyTag \"Peak\" off 1 of 1 track.\n"
    );
    told_of_one_change(&mut watcher);
    assert_eq!(tags_of(sandbox, 2), ["Warm-up", "Acid"]);
    assert_eq!(tags_of(sandbox, 1), ["Peak", "Dark"]);

    let run = sandbox.cli(&["mytags", "assign", &made.acid, &track_id(2), &track_id(6)]);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(
        common::without_run(&run.stdout),
        "Put MyTag \"Acid\" on 1 of 2 tracks.\n"
    );
}

#[test]
fn deleting_a_tag_takes_it_off_every_track() {
    let made = prepared();
    let sandbox = &made.sandbox;
    let mut watcher = watcher(sandbox);

    let deleted = json_of(sandbox, &["--json", "mytags", "delete", MY_TAG_PEAK]);

    assert_eq!(deleted["my_tag"]["id"], MY_TAG_PEAK);
    assert_eq!(deleted["my_tag"]["name"], "Peak");
    assert_eq!(deleted["tracks"], 2);
    assert_eq!(deleted["generation"], told_of_one_change(&mut watcher));
    assert_eq!(
        vocabulary(sandbox),
        named(&[
            ("Situation", &[("Warm-up", 1)]),
            ("Mood", &[("Dark", 2), ("Uplifting", 0)]),
            ("Sound", &[("Acid", 1)]),
            ("Sub-genre", &[]),
        ])
    );
    assert_eq!(tags_of(sandbox, 1), ["Dark"]);
    assert_eq!(tags_of(sandbox, 2), ["Warm-up", "Acid"]);
    assert_eq!(
        refusal(sandbox, &["mytags", "tracks", MY_TAG_PEAK]),
        "my_tag_not_found"
    );

    let run = sandbox.cli(&["mytags", "delete", &made.uplifting]);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(
        common::without_run(&run.stdout),
        format!(
            "Deleted MyTag {} \"Uplifting\" from Mood; no track carried it.\n",
            made.uplifting
        )
    );
}

#[test]
fn merging_moves_every_track_to_the_other_tag_and_removes_the_first() {
    let made = prepared();
    let sandbox = &made.sandbox;
    let mut watcher = watcher(sandbox);

    // Peak is on tracks 1 and 2, Warm-up on track 2 already.
    let merged = json_of(
        sandbox,
        &["--json", "mytags", "merge", MY_TAG_PEAK, MY_TAG_WARM_UP],
    );

    assert_eq!(merged["from"]["name"], "Peak");
    assert_eq!(merged["into"]["name"], "Warm-up");
    assert_eq!(merged["tracks"], 2);
    // One edit, so one event.
    assert_eq!(merged["generation"], told_of_one_change(&mut watcher));
    assert_eq!(
        vocabulary(sandbox),
        named(&[
            ("Situation", &[("Warm-up", 2)]),
            ("Mood", &[("Dark", 2), ("Uplifting", 0)]),
            ("Sound", &[("Acid", 1)]),
            ("Sub-genre", &[]),
        ])
    );
    assert_eq!(tags_of(sandbox, 1), ["Warm-up", "Dark"]);
    assert_eq!(tags_of(sandbox, 2), ["Warm-up", "Acid"]);
    assert_eq!(tags_of(sandbox, 3), ["Dark"]);

    // Across categories, by hand.
    let run = sandbox.cli(&["mytags", "merge", &made.dark, &made.acid]);
    assert!(run.status.success(), "{}", run.stderr);
    assert_eq!(
        common::without_run(&run.stdout),
        "Merged MyTag \"Dark\" (Mood) into \"Acid\" (Sound): 2 tracks moved.\n"
    );
    assert_eq!(tags_of(sandbox, 1), ["Warm-up", "Acid"]);
    assert_eq!(tags_of(sandbox, 3), ["Acid"]);
}

#[test]
fn with_the_comments_option_each_edit_rewrites_the_comments_as_the_window_does() {
    let made = prepared();
    let sandbox = &made.sandbox;
    for (index, comment) in [(4, "Vinyl rip"), (5, "Keep")] {
        sandbox.detail_track(
            index,
            &Details {
                comment,
                ..Details::default()
            },
        );
    }
    let mut watcher = watcher(sandbox);
    let mut edit = |args: &[&str]| {
        let run = sandbox.cli(args);
        assert!(run.status.success(), "{}", run.stderr);
        told_of_one_change(&mut watcher);
    };

    // A tag put on a track is written after its comment, in the order the
    // tags were put on.
    edit(&["mytags", "assign", "--comments", &made.dark, &track_id(4)]);
    assert_eq!(comment_of(sandbox, 4), "Vinyl rip /* Dark */");
    edit(&["mytags", "assign", "--comments", &made.acid, &track_id(4)]);
    assert_eq!(comment_of(sandbox, 4), "Vinyl rip /* Dark / Acid */");

    // A rename reaches every track that carries the tag, one with no
    // comment of its own included.
    edit(&["mytags", "rename", "--comments", &made.dark, "Moody"]);
    assert_eq!(comment_of(sandbox, 4), "Vinyl rip /* Moody / Acid */");
    assert_eq!(comment_of(sandbox, 3), " /* Moody */");

    edit(&["mytags", "unassign", "--comments", &made.acid, &track_id(4)]);
    assert_eq!(comment_of(sandbox, 4), "Vinyl rip /* Moody */");

    edit(&["mytags", "merge", "--comments", &made.dark, &made.uplifting]);
    assert_eq!(comment_of(sandbox, 4), "Vinyl rip /* Uplifting */");
    assert_eq!(comment_of(sandbox, 3), " /* Uplifting */");

    edit(&["mytags", "delete", "--comments", &made.uplifting]);
    assert_eq!(comment_of(sandbox, 4), "Vinyl rip");
    assert_eq!(comment_of(sandbox, 3), "");

    // Without the option a comment is left alone.
    edit(&["mytags", "assign", MY_TAG_PEAK, &track_id(5)]);
    assert_eq!(comment_of(sandbox, 5), "Keep");
}

#[test]
fn an_invalid_request_fails_with_a_stable_code_and_changes_nothing() {
    let made = prepared();
    let sandbox = &made.sandbox;
    let mut watcher = watcher(sandbox);
    let (mood, dark, uplifting) = (
        made.mood.as_str(),
        made.dark.as_str(),
        made.uplifting.as_str(),
    );
    let (one, four) = (track_id(1), track_id(4));

    for (args, code) in [
        // rekordbox fixes the categories: there is no fifth.
        (
            vec!["mytags", "create-category", "Energy"],
            "my_tag_category_limit",
        ),
        // An unknown category, and a tag where a category is wanted.
        (
            vec!["mytags", "create", "12345", "Hypnotic"],
            "my_tag_category_not_found",
        ),
        (
            vec!["mytags", "create", dark, "Hypnotic"],
            "my_tag_category_not_found",
        ),
        // A name the category already has.
        (vec!["mytags", "create", mood, "Dark"], "my_tag_name_taken"),
        (
            vec!["mytags", "rename", uplifting, "Dark"],
            "my_tag_name_taken",
        ),
        (vec!["mytags", "rename", mood, "Sound"], "my_tag_name_taken"),
        (vec!["mytags", "create", mood, ""], "invalid_my_tag_name"),
        (vec!["mytags", "rename", dark, ""], "invalid_my_tag_name"),
        // An unknown tag.
        (
            vec!["mytags", "rename", "12345", "Hypnotic"],
            "my_tag_not_found",
        ),
        (vec!["mytags", "move", "12345", "1"], "my_tag_not_found"),
        (vec!["mytags", "delete", "12345"], "my_tag_not_found"),
        (vec!["mytags", "merge", "12345", dark], "my_tag_not_found"),
        (vec!["mytags", "merge", dark, "12345"], "my_tag_not_found"),
        (vec!["mytags", "assign", "12345", &one], "my_tag_not_found"),
        (
            vec!["mytags", "unassign", "12345", &one],
            "my_tag_not_found",
        ),
        // A category is not a tag: it cannot be deleted, merged or assigned.
        (vec!["mytags", "delete", mood], "my_tag_not_found"),
        (vec!["mytags", "merge", mood, dark], "my_tag_not_found"),
        (vec!["mytags", "assign", mood, &one], "my_tag_not_found"),
        (
            vec!["mytags", "merge", dark, dark],
            "my_tag_merge_into_itself",
        ),
        // One unknown track refuses the whole request.
        (
            vec!["mytags", "assign", uplifting, &four, "99999"],
            "track_not_found",
        ),
        (
            vec!["mytags", "unassign", dark, &one, "99999"],
            "track_not_found",
        ),
    ] {
        assert_eq!(refusal(sandbox, &args), code, "{args:?}");
    }

    told_nothing(&mut watcher);
    assert_eq!(vocabulary(sandbox), as_prepared());
    assert_eq!(tags_of(sandbox, 1), ["Peak", "Dark"]);
    assert_eq!(tags_of(sandbox, 4), Vec::<String>::new());

    // By hand the refusal is one line on standard error.
    let run = sandbox.cli(&["mytags", "create-category", "Energy"]);
    assert!(!run.status.success());
    assert_eq!(common::without_run(&run.stdout), "");
    assert!(run.stderr.contains("four"), "{}", run.stderr);

    // The same name in another category is not a clash.
    let run = sandbox.cli(&["mytags", "create", mood, "Peak"]);
    assert!(run.status.success(), "{}", run.stderr);
}
