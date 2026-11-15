//! MyTag definitions in MyTag Projection (wbx-14.14), through the real CLI
//! and a service on a fixture Library, with a second client watching: the
//! MyTags the Tag schema has and the Library lacks are made, and a deck
//! tag's label and place follow the schema only for the MyTags a
//! projection made itself; everything the DJ made is kept and reported.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
use crate::common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use common::Sandbox;
use serde_json::{json, Value};

fn ok(s: &Sandbox, args: &[&str]) -> Value {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    let result = s.cli(&words);
    assert!(
        result.status.success(),
        "{args:?}: {}{}",
        result.stdout,
        result.stderr
    );
    result.json()
}

fn refused(s: &Sandbox, args: &[&str]) -> Value {
    let mut words = vec!["--json"];
    words.extend_from_slice(args);
    let result = s.cli(&words);
    assert!(!result.status.success(), "{args:?}: {}", result.stdout);
    result.error_json()["error"].clone()
}

fn write(s: &Sandbox, file: &str, text: &str) {
    std::fs::create_dir_all(s.sidecar_dir()).unwrap();
    std::fs::write(s.sidecar_dir().join(file), text).unwrap();
}

fn fact(s: &Sandbox, track: &str, kind: &str, values: &[&str]) {
    let mut args = vec!["facts", "record", track, kind];
    args.extend_from_slice(values);
    args.extend_from_slice(&["--source", "agent:tagger", "--confidence", "0.9"]);
    ok(s, &args);
}

/// The database files of the Library and the Sidecar, byte for byte.
fn stored_bytes(s: &Sandbox) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for db in [
        s.master_db().to_path_buf(),
        s.sidecar_dir().join("sidecar.db"),
    ] {
        for suffix in ["", "-wal", "-journal"] {
            let path = PathBuf::from(format!("{}{suffix}", db.display()));
            if path.exists() {
                files.insert(path.clone(), std::fs::read(path).unwrap());
            }
        }
    }
    files
}

/// Each category's tags, by name, in the Library's order.
fn vocabulary(s: &Sandbox) -> Vec<(String, Vec<String>)> {
    ok(s, &["mytags", "list"])["categories"]
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
                    .map(|tag| tag["name"].as_str().unwrap().to_owned())
                    .collect(),
            )
        })
        .collect()
}

fn tags_in(s: &Sandbox, category: &str) -> Vec<String> {
    vocabulary(s)
        .into_iter()
        .find(|(name, _)| name == category)
        .map(|(_, tags)| tags)
        .unwrap_or_default()
}

/// The id of the MyTag named `name` in `category`.
fn tag_id(s: &Sandbox, category: &str, name: &str) -> String {
    ok(s, &["mytags", "list"])["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == category)
        .and_then(|c| {
            c["tags"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
        })
        .map(|t| t["id"].as_str().unwrap().to_owned())
        .unwrap_or_else(|| panic!("no {category}: {name}"))
}

fn tags_of(s: &Sandbox, track: &str) -> Vec<String> {
    let mut tags: Vec<String> = ok(s, &["mytags", "show", track])["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| {
            format!(
                "{}: {}",
                tag["category"].as_str().unwrap(),
                tag["name"].as_str().unwrap()
            )
        })
        .collect();
    tags.sort();
    tags
}

/// `(change, category, label, reason)` of each definition under `key`.
fn definitions(report: &Value, key: &str) -> Vec<(String, String, String, String)> {
    report[key]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|d| {
            (
                d["change"].as_str().unwrap().to_owned(),
                d["category"].as_str().unwrap().to_owned(),
                d["label"].as_str().unwrap().to_owned(),
                d["reason"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

fn d(change: &str, category: &str, label: &str, reason: &str) -> (String, String, String, String) {
    (change.into(), category.into(), label.into(), reason.into())
}

/// The DJ's MyTags: Mood with Dark and Uplifting, made by hand.
fn by_hand() -> Sandbox {
    let s = Sandbox::new();
    let [_, mood, _sound, _sub_genre] = s.four_categories();
    let dark = s.create_my_tag_in(&mood, "Dark");
    s.create_my_tag_in(&mood, "Uplifting");
    s.detail_track(
        1,
        &common::Details {
            my_tags: &[dark.as_str()],
            ..common::Details::default()
        },
    );
    s
}

/// A Mood dimension with `terms` (`id = name` pairs, in order) and the
/// deck's `order` for it.
fn schema(terms: &[(&str, &str)], order: &[&str]) -> String {
    let terms: String = terms
        .iter()
        .map(|(id, name)| format!("  {{ id = \"{id}\", name = \"{name}\" }},\n"))
        .collect();
    let order = if order.is_empty() {
        String::new()
    } else {
        let ids: Vec<String> = order.iter().map(|id| format!("\"{id}\"")).collect();
        format!(", order = [{}]", ids.join(", "))
    };
    format!(
        "version = 1\n\n[dimensions.mood]\nterms = [\n{terms}]\n\n[deck]\ncategories = [\n  {{ dimension = \"mood\", label = \"Mood\"{order} }},\n]\n"
    )
}

const MOOD: &[(&str, &str)] = &[
    ("dark", "Dark"),
    ("uplifting", "Uplifting"),
    ("euphoric", "Euphoric"),
];

/// A MyTag the schema has and the Library lacks is listed by the dry run
/// (which writes nothing) and made by the apply, with the assignments that
/// use it, in ONE Run, ONE Ledger entry and ONE `library_changed`; then
/// there is nothing left to do.
#[test]
fn projecting_makes_the_mytags_the_schema_has_with_their_assignments_as_one_run() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["euphoric"]);
    let before = stored_bytes(&s);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(stored_bytes(&s), before, "a dry run writes nothing");
    assert_eq!(
        definitions(&dry, "definitions"),
        [d("create_tag", "Mood", "Euphoric", "")],
        "{dry}"
    );
    let change = &dry["changes"][0];
    assert_eq!(change["label"], "Euphoric", "{dry}");
    assert_eq!(change["change"], "add");
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting"]);

    let runs = ok(&s, &["runs", "list"])["total"].as_i64().unwrap();
    let mut watcher = s.client();
    watcher.subscribe().unwrap();
    let applied = ok(
        &s,
        &[
            "project",
            "--mytags",
            "--expect",
            dry["plan"].as_str().unwrap(),
        ],
    );
    assert_eq!(applied["definitions"], dry["definitions"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Euphoric"]);
    assert_eq!(tags_of(&s, "10004"), ["Mood: Euphoric"]);
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs + 1);
    let shown = ok(&s, &["runs", "show", &applied["run"]["id"].to_string()]);
    let entries = shown["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "one Ledger entry: {shown}");
    assert_eq!(entries[0]["reversible"], true);
    let mut heard = Vec::new();
    while let Some(event) = watcher.next_event(Duration::from_millis(500)).unwrap() {
        heard.push(event.event);
    }
    assert_eq!(heard, ["library_changed"]);

    let again = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(again["definitions"], json!([]), "{again}");
    assert_eq!(again["changes"], json!([]), "{again}");
}

/// Definitions are a Library edit like any other: refused under Library
/// Protection with nothing made.
#[test]
fn library_protection_refuses_making_a_mytag() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    ok(&s, &["protection", "on"]);
    let error = refused(&s, &["project", "--mytags"]);
    assert_eq!(error["code"], "library_protected", "{error}");
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting"]);
}

/// A label changed in the schema renames the MyTag only when a projection
/// made it and nobody wrote it since; the DJ's MyTag of a relabelled term
/// is kept, reported, and nothing is made in its place.
#[test]
fn a_relabelled_term_renames_only_the_projections_own_mytag() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    fact(&s, "10004", "mood", &["euphoric"]);
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    ok(&s, &["project", "--mytags"]);
    let euphoric = tag_id(&s, "Mood", "Euphoric");

    write(
        &s,
        "tag-schema.toml",
        &schema(
            &[
                ("dark", "Darkest"),
                ("uplifting", "Uplifting"),
                ("euphoric", "Euphoria"),
            ],
            &[],
        ),
    );
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(
        definitions(&dry, "definitions"),
        [d("rename_tag", "Mood", "Euphoria", "")],
        "{dry}"
    );
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("rename_tag", "Mood", "Darkest", "set_by_dj")],
        "{dry}"
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Euphoria"]);
    assert_eq!(tag_id(&s, "Mood", "Euphoria"), euphoric, "the same MyTag");
    assert_eq!(tags_of(&s, "10004"), ["Mood: Euphoria"]);
    assert_eq!(tags_of(&s, "10001"), ["Mood: Dark"]);

    // The DJ renames the projection's MyTag: theirs from then on.
    ok(&s, &["mytags", "rename", &euphoric, "Joy"]);
    write(
        &s,
        "tag-schema.toml",
        &schema(
            &[
                ("dark", "Dark"),
                ("uplifting", "Uplifting"),
                ("euphoric", "Bliss"),
            ],
            &[],
        ),
    );
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("rename_tag", "Mood", "Bliss", "written_since")],
        "{dry}"
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Joy"]);
}

/// The deck's order places the projection's own MyTags; the DJ's keep
/// their order among themselves, and an order that would move them is kept
/// and reported, as is a deck order of categories the Library does not
/// have.
#[test]
fn the_deck_order_places_only_the_projections_own_mytags() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &["euphoric"]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    let listed = definitions(&dry, "definitions");
    assert_eq!(
        listed,
        [
            d("create_tag", "Mood", "Euphoric", ""),
            d("order_tags", "Mood", "", "")
        ],
        "{dry}"
    );
    assert_eq!(
        dry["definitions"][1]["order"],
        json!(["Euphoric", "Dark", "Uplifting"])
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Euphoric", "Dark", "Uplifting"]);

    // Uplifting before Dark would move the DJ's own: kept.
    write(&s, "tag-schema.toml", &schema(MOOD, &["uplifting"]));
    let before = vocabulary(&s);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("order_tags", "Mood", "", "set_by_dj")],
        "{dry}"
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(vocabulary(&s), before);
}


// ---------------------------------------------------------------------
// Review round 1 (codex/review-wbx-14.14-round1.md): the reviewer's
// failing sequences, one test each, and the regressions it listed.
// ---------------------------------------------------------------------

/// Runs a CLI command whose service, started by it, has `env` set.
fn with_env(s: &Sandbox, env: &[(&str, &str)], args: &[&str]) -> common::Run {
    let mut command = s.command();
    for (key, value) in env {
        command.env(key, value);
    }
    command.arg("--json").args(args);
    s.run(&mut command)
}

fn service_pid(s: &Sandbox) -> Option<u64> {
    let mut client =
        wreckordbox_service::Client::connect(&s.config().socket_path(s.location())).ok()?;
    client.status().ok().map(|status| u64::from(status.pid))
}

fn wait_gone(pid: Option<u64>) {
    let Some(pid) = pid else { return };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while common::alive(pid) {
        assert!(std::time::Instant::now() < deadline, "the service did not stop");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Stops the service here and waits until its process is gone.
fn stop_service(s: &Sandbox) {
    let pid = service_pid(s);
    if let Ok(mut client) =
        wreckordbox_service::Client::connect(&s.config().socket_path(s.location()))
    {
        let _ = client.stop();
    }
    wait_gone(pid);
}

/// Kills the service here as a crash would, and waits until it is gone.
fn kill_service(s: &Sandbox) {
    let pid = service_pid(s).expect("a service is running");
    std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    wait_gone(Some(pid));
}

fn waits_for(file: &std::path::Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !file.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "{} never appeared",
            file.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A rename made by another program (rekordbox) straight in the Library.
fn renamed_elsewhere(s: &Sandbox, id: &str, name: &str) {
    s.edit(|writer| {
        writer.rename_my_tag(id, name).unwrap();
    });
}

const BLISS: &[(&str, &str)] = &[
    ("dark", "Dark"),
    ("uplifting", "Uplifting"),
    ("euphoric", "Bliss"),
];

/// The DJ's MyTags and a projection's own "Euphoric", made on track 4.
fn projected_euphoric() -> (Sandbox, String) {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["euphoric"]);
    ok(&s, &["project", "--mytags"]);
    let id = tag_id(&s, "Mood", "Euphoric");
    (s, id)
}

/// Finding 1: the DJ renaming a MyTag to the name it already has is the
/// DJ's act, recorded though nothing changes: the projection never
/// renames it again. A resend of the rename that does not say it asserts
/// changes nothing about ownership.
#[test]
fn a_djs_rename_to_the_same_name_ends_the_projections_ownership() {
    let (s, euphoric) = projected_euphoric();
    // A client resending the rename without asserting: still the
    // projection's.
    s.client()
        .call::<Value>(
            "mytags.rename",
            json!({"id": euphoric, "name": "Euphoric"}),
        )
        .unwrap();
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(
        definitions(&dry, "definitions"),
        [d("rename_tag", "Mood", "Bliss", "")],
        "{dry}"
    );

    // The DJ renames it to the same name, deliberately.
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    ok(&s, &["mytags", "rename", &euphoric, "Euphoric"]);
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("rename_tag", "Mood", "Bliss", "set_by_dj")],
        "{dry}"
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Euphoric"]);
}

/// Finding 2, first race: a rename by another program after the apply's
/// last check and before its transaction refuses the whole apply; the
/// rename stands.
#[test]
fn a_definition_renamed_while_the_apply_is_held_refuses_it() {
    let (s, euphoric) = projected_euphoric();
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    stop_service(&s);
    let hold = s.path().join("hold");
    std::fs::create_dir_all(&hold).unwrap();
    let apply = {
        let command = {
            let mut command = s.command();
            command
                .env("WRECKORDBOX_MYTAG_PROJECTION_HOLD", &hold)
                .args(["--json", "project", "--mytags"]);
            command
        };
        let mut command = command;
        std::thread::spawn(move || command.output().unwrap())
    };
    waits_for(&hold.join("checked"));
    renamed_elsewhere(&s, &euphoric, "Joy");
    std::fs::write(hold.join("write"), b"").unwrap();
    let out = apply.join().unwrap();
    assert!(!out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let text = String::from_utf8_lossy(&out.stderr).to_string()
        + &String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("projection_changed"), "{text}");
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Joy"]);
}

/// Finding 2, second race: a rename by another program after the apply
/// committed and before it kept its marks is never taken for the
/// projection's own: a later relabel keeps it.
#[test]
fn a_definition_renamed_between_the_commit_and_the_marks_stays_the_djs() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["euphoric"]);
    stop_service(&s);
    let hold = s.path().join("hold");
    std::fs::create_dir_all(&hold).unwrap();
    let apply = {
        let mut command = s.command();
        command
            .env("WRECKORDBOX_MYTAG_PROJECTION_COMMIT_HOLD", &hold)
            .args(["--json", "project", "--mytags"]);
        std::thread::spawn(move || command.output().unwrap())
    };
    waits_for(&hold.join("committed"));
    let euphoric = {
        let mut id = String::new();
        s.edit(|writer| {
            let conn = writer.library().connection();
            id = conn
                .query_row(
                    "SELECT ID FROM djmdMyTag WHERE Name = 'Euphoric' AND rb_local_deleted = 0",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
        });
        id
    };
    renamed_elsewhere(&s, &euphoric, "Joy");
    std::fs::write(hold.join("mark"), b"").unwrap();
    assert!(apply.join().unwrap().status.success());
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("rename_tag", "Mood", "Bliss", "written_since")],
        "{dry}"
    );
}

/// Finding 3: a linked MyTag the DJ renamed is the term's still, even when
/// its new name is another term's label: nothing is made again, the DJ's
/// MyTag is not handed to the other term, and the conflict is reported.
#[test]
fn a_renamed_definition_whose_name_collides_with_another_term_stays_protected() {
    let (s, euphoric) = projected_euphoric();
    ok(&s, &["mytags", "rename", &euphoric, "Joy"]);
    write(
        &s,
        "tag-schema.toml",
        &schema(
            &[
                ("dark", "Dark"),
                ("uplifting", "Uplifting"),
                ("euphoric", "Euphoric"),
                ("joy", "Joy"),
            ],
            &[],
        ),
    );
    fact(&s, "10005", "mood", &["joy"]);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    let mut kept = definitions(&dry, "definitions_kept");
    kept.sort();
    assert_eq!(
        kept,
        [
            d("create_tag", "Mood", "Joy", "name_conflict"),
            d("rename_tag", "Mood", "Euphoric", "set_by_dj"),
        ],
        "{dry}"
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Joy"]);
    // Track 5's joy has nowhere to go; the DJ's MyTag is not given to it.
    assert_eq!(tags_of(&s, "10005"), Vec::<String>::new());
}

/// Finding 5: the apply dies after its Library commit and before it kept
/// its marks; the DJ then deletes the MyTag it made. It is never made
/// again.
#[test]
fn a_mytag_made_by_an_apply_that_died_before_its_marks_is_not_made_again_once_deleted() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["euphoric"]);
    stop_service(&s);
    let hold = s.path().join("hold");
    std::fs::create_dir_all(&hold).unwrap();
    let mut apply = s
        .command()
        .env("WRECKORDBOX_MYTAG_PROJECTION_COMMIT_HOLD", &hold)
        .args(["--json", "project", "--mytags"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    waits_for(&hold.join("committed"));
    // The client first, so it cannot start another service and ask again.
    apply.kill().unwrap();
    apply.wait().unwrap();
    kill_service(&s);
    let euphoric = tag_id(&s, "Mood", "Euphoric");
    ok(&s, &["mytags", "delete", &euphoric]);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert!(
        definitions(&dry, "definitions_kept").contains(&d("create_tag", "Mood", "Euphoric", "removed")),
        "{dry}"
    );
}

/// Finding 5, the other side: an apply that died before its commit made
/// nothing, but whether it did is uncertain until the DJ clears its
/// Barrier: no MyTag is made behind it.
#[test]
fn an_apply_that_died_before_its_commit_bars_making_mytags_until_its_barrier_is_cleared() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["euphoric"]);
    stop_service(&s);
    let hold = s.path().join("hold");
    std::fs::create_dir_all(&hold).unwrap();
    let mut apply = s
        .command()
        .env("WRECKORDBOX_MYTAG_PROJECTION_HOLD", &hold)
        .args(["--json", "project", "--mytags"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    waits_for(&hold.join("checked"));
    // The client first, so it cannot start another service and ask again.
    apply.kill().unwrap();
    apply.wait().unwrap();
    kill_service(&s);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting"]);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("create_tag", "Mood", "Euphoric", "provenance_incomplete")],
        "{dry}"
    );
    let barriers = ok(&s, &["projection", "barriers"]);
    let seq = barriers["barriers"][0]["seq"].to_string();
    ok(&s, &["projection", "acknowledge", &seq, "--source", "dj"]);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(
        definitions(&dry, "definitions"),
        [d("create_tag", "Mood", "Euphoric", "")],
        "{dry}"
    );
}

/// Finding 6: a token names the definitions as they were read; after
/// another projection renamed the MyTag, the old token is refused even
/// once the schema says again what it said.
#[test]
fn a_stale_plan_token_for_a_definition_is_refused() {
    let (s, _) = projected_euphoric();
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    let first = ok(&s, &["project", "--mytags", "--dry-run"]);
    let token = first["plan"].as_str().unwrap().to_owned();
    write(
        &s,
        "tag-schema.toml",
        &schema(
            &[
                ("dark", "Dark"),
                ("uplifting", "Uplifting"),
                ("euphoric", "Calm"),
            ],
            &[],
        ),
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Calm"]);
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    let error = refused(&s, &["project", "--mytags", "--expect", &token]);
    assert_eq!(error["code"], "projection_changed", "{error}");
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Calm"]);
}

/// Finding 7, first half: a Barrier bounded to tracks (a hand rating cut
/// off after its write) cannot have changed a definition: the
/// projection's own MyTag is still renamed.
#[test]
fn a_track_only_barrier_does_not_bar_renaming_the_projections_mytag() {
    let (s, _) = projected_euphoric();
    stop_service(&s);
    let crashed = with_env(
        &s,
        &[("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")],
        &["tracks", "rate", "10002", "4", "--source", "dj"],
    );
    assert!(!crashed.status.success());
    let barriers = ok(&s, &["projection", "barriers"]);
    assert_eq!(barriers["barriers"][0]["tracks"], json!(["10002"]), "{barriers}");
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(
        definitions(&dry, "definitions"),
        [d("rename_tag", "Mood", "Bliss", "")],
        "{dry}"
    );
}

/// Finding 7, second half: an apply that changes definitions and dies is
/// a Barrier over everything, even when it was asked for one track.
#[test]
fn a_definition_apply_that_dies_is_a_barrier_over_everything_whatever_its_track_filter() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["euphoric"]);
    stop_service(&s);
    let crashed = with_env(
        &s,
        &[("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")],
        &["project", "--mytags", "--track", "10004"],
    );
    assert!(!crashed.status.success());
    let barriers = ok(&s, &["projection", "barriers"]);
    let listed = barriers["barriers"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{barriers}");
    assert_eq!(listed[0]["tracks"], Value::Null, "{barriers}");
}

/// An unbounded Barrier after the projection's write (a DJ's MyTag rename
/// cut off after its write) bars renaming the projection's MyTag.
#[test]
fn an_unbounded_barrier_after_the_projections_write_bars_renaming_its_mytag() {
    let (s, _) = projected_euphoric();
    let dark = tag_id(&s, "Mood", "Dark");
    stop_service(&s);
    let crashed = with_env(
        &s,
        &[("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")],
        &["mytags", "rename", &dark, "Darker"],
    );
    assert!(!crashed.status.success());
    write(
        &s,
        "tag-schema.toml",
        &schema(
            &[
                ("dark", "Darker"),
                ("uplifting", "Uplifting"),
                ("euphoric", "Bliss"),
            ],
            &[],
        ),
    );
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert!(
        definitions(&dry, "definitions_kept")
            .contains(&d("rename_tag", "Mood", "Bliss", "provenance_incomplete")),
        "{dry}"
    );
}

/// The mark is evidence only for its confirmed Ledger entry: an apply that
/// dies after its write, before its entry, leaves its MyTag the DJ's.
#[test]
fn a_mytag_made_by_an_apply_whose_entry_was_never_confirmed_is_never_renamed() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["euphoric"]);
    stop_service(&s);
    let crashed = with_env(
        &s,
        &[("WRECKORDBOX_LEDGER_FAULTS", "abort_after_write")],
        &["project", "--mytags"],
    );
    assert!(!crashed.status.success());
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Euphoric"]);
    // The Barrier cleared, the mark still proves nothing.
    let barriers = ok(&s, &["projection", "barriers"]);
    let seq = barriers["barriers"][0]["seq"].to_string();
    ok(&s, &["projection", "acknowledge", &seq, "--source", "dj"]);
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert_eq!(definitions(&dry, "definitions_kept").len(), 1, "{dry}");
    assert_eq!(dry["definitions_kept"][0]["change"], "rename_tag", "{dry}");
}

/// Finding 8: an apply with nothing to change writes nothing at all: not
/// the Library, not the Sidecar.
#[test]
fn an_apply_with_nothing_to_change_writes_nothing() {
    let s = by_hand();
    write(
        &s,
        "tag-schema.toml",
        &schema(&[("dark", "Dark"), ("uplifting", "Uplifting")], &[]),
    );
    fact(&s, "10001", "mood", &["dark"]);
    let runs = ok(&s, &["runs", "list"])["total"].clone();
    let before = stored_bytes(&s);
    let applied = ok(&s, &["project", "--mytags"]);
    assert_eq!(applied["changes"], json!([]), "{applied}");
    assert_eq!(applied["definitions"], json!([]), "{applied}");
    assert_eq!(stored_bytes(&s), before, "a no-op apply wrote");
    assert_eq!(ok(&s, &["runs", "list"])["total"], runs);
}

/// The DJ reordering MyTags, the projection's among them, makes the
/// projection's the DJ's, even one whose own place number did not change.
#[test]
fn a_djs_reorder_ends_the_projections_ownership_of_its_mytags() {
    let (s, euphoric) = projected_euphoric();
    let dark = tag_id(&s, "Mood", "Dark");
    let uplifting = tag_id(&s, "Mood", "Uplifting");
    // Euphoric keeps the third place.
    ok(&s, &["mytags", "reorder", &uplifting, &dark, &euphoric]);
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert!(
        definitions(&dry, "definitions_kept")
            .contains(&d("rename_tag", "Mood", "Bliss", "set_by_dj")),
        "{dry}"
    );
}

/// The DJ deleting the projection's MyTag: it is never made again.
#[test]
fn a_mytag_the_dj_deleted_is_never_made_again() {
    let (s, euphoric) = projected_euphoric();
    ok(&s, &["mytags", "delete", &euphoric]);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    assert_eq!(
        definitions(&dry, "definitions_kept"),
        [d("create_tag", "Mood", "Euphoric", "removed")],
        "{dry}"
    );
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting"]);
}

/// The DJ deletes the projection's MyTag and makes one of the same name by
/// hand: that one is the DJ's, used for the term and never renamed, and no
/// second one is made.
#[test]
fn a_mytag_the_dj_deleted_and_made_again_is_the_djs() {
    let (s, euphoric) = projected_euphoric();
    let mood = ok(&s, &["mytags", "list"])["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Mood")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(&s, &["mytags", "delete", &euphoric]);
    ok(&s, &["mytags", "create", &mood, "Euphoric"]);
    let again = tag_id(&s, "Mood", "Euphoric");
    assert_ne!(again, euphoric);
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    write(&s, "tag-schema.toml", &schema(BLISS, &[]));
    let dry = ok(&s, &["project", "--mytags", "--dry-run"]);
    assert_eq!(definitions(&dry, "definitions"), [], "{dry}");
    ok(&s, &["project", "--mytags"]);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting", "Euphoric"]);
    assert_eq!(tag_id(&s, "Mood", "Euphoric"), again);
}

/// A Library on a Stick: the apply that would make a MyTag is refused and
/// not a byte of the Library changes.
#[test]
fn an_apply_on_a_library_on_a_stick_is_refused_and_makes_no_mytag() {
    let s = by_hand();
    write(&s, "tag-schema.toml", &schema(MOOD, &[]));
    write(&s, "projection-policy.toml", "[mytags]\napply = 0.5\n");
    fact(&s, "10004", "mood", &["euphoric"]);
    let folder = s.master_db().parent().unwrap().to_path_buf();
    std::fs::write(
        folder.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let name = folder.file_name().unwrap().to_str().unwrap().to_owned();
    ok(&s, &["stick", "unlock", folder.to_str().unwrap(), "--name", &name]);
    let library = std::fs::read(s.master_db()).unwrap();
    let error = refused(&s, &["project", "--mytags"]);
    assert_eq!(error["code"], "sidecar_on_stick", "{error}");
    assert_eq!(std::fs::read(s.master_db()).unwrap(), library);
    assert_eq!(tags_in(&s, "Mood"), ["Dark", "Uplifting"]);
}
