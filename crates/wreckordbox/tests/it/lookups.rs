//! `wreckordbox lookup`: describing a track for an agent's search and
//! checking a candidate the agent found against it (wbx-37, ADR 0005).
//! The real binary against a Library service on a throwaway Library and
//! Sidecar, and what a second connected client is told.
//!
//! Wreckordbox never fetches anything: every candidate here is handed to
//! it, as an agent with its own network access would hand one over.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::Sandbox;
use lofty::config::WriteOptions;
use lofty::tag::{Accessor, TagExt};
use rbl_db::write::TrackField;
use serde_json::{json, Value};

/// Runs a command that must succeed and answers its JSON result.
fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    assert_eq!(run.stderr, "", "{args:?}");
    run.json()
}

/// Runs a command that must fail and answers its JSON error.
fn refused(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(!run.status.success(), "{args:?}: {}", run.stdout);
    assert_eq!(run.stdout, "", "{args:?}");
    run.error_json()["error"].clone()
}

/// Runs a command that must succeed and answers its text.
fn text(sandbox: &Sandbox, args: &[&str]) -> String {
    let run = sandbox.cli(args);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.stdout
}

const ADDRESS: &str = "https://shop.example.test/release/4471/night-signal";
const LOOKED_UP: &str = "2026-11-10";

/// Imports a 30-second file, tags the file itself as a download from a
/// shop would be, and gives the track in the Library the DJ's own details,
/// which differ from the file's in places. Answers the track's identifier
/// and the audio file's path.
fn night_signal(sandbox: &Sandbox) -> (String, PathBuf) {
    let id = sandbox.import_track("night-signal.wav", 30);
    let file = sandbox.path().join("audio").join("night-signal.wav");
    let mut tag = lofty::iff::wav::RiffInfoList::default();
    tag.set_title("Night Signal (Extended Mix)".to_owned());
    tag.set_artist("Kessler, Vane".to_owned());
    tag.set_genre("Tech House".to_owned());
    tag.save_to_path(&file, WriteOptions::default()).unwrap();
    sandbox.edit(|writer| {
        writer.set_field(&id, TrackField::Title, "Night Signal").unwrap();
        writer
            .set_field(&id, TrackField::Artist, "Kessler & Vane")
            .unwrap();
        writer.set_field(&id, TrackField::Label, "Lowfield").unwrap();
        writer.set_field(&id, TrackField::Year, "2019").unwrap();
        writer
            .library()
            .connection()
            .execute(
                "UPDATE djmdContent SET Subtitle = 'Extended Mix', ReleaseDate = '2019-04-12' WHERE ID = ?1",
                [&id],
            )
            .unwrap();
    });
    (id, file)
}

#[test]
fn a_track_is_described_for_a_search_as_structured_data() {
    let s = Sandbox::new();
    let (id, file) = night_signal(&s);

    let described = ok(&s, &["lookup", "describe", &id]);

    assert_eq!(described["track"], json!(id));
    assert_eq!(described["artist"], "Kessler & Vane");
    assert_eq!(described["artists"], json!(["Kessler", "Vane"]));
    assert_eq!(described["title"], "Night Signal");
    assert_eq!(described["mix_name"], "Extended Mix");
    assert_eq!(described["duration_seconds"], 30);
    assert_eq!(described["label"], "Lowfield");
    assert_eq!(described["release_date"], "2019-04-12");
    assert_eq!(described["year"], 2019);
    assert_eq!(
        described["search"],
        "Kessler & Vane - Night Signal (Extended Mix)"
    );
    // The file's own tags, as the file has them, not as the Library does.
    assert_eq!(described["file"]["path"], json!(file.to_str().unwrap()));
    assert_eq!(described["file"]["problem"], Value::Null);
    let tags = &described["file"]["tags"];
    assert_eq!(tags["title"], "Night Signal (Extended Mix)");
    assert_eq!(tags["artist"], "Kessler, Vane");
    assert_eq!(tags["genre"], "Tech House");
    assert_eq!(tags["duration_seconds"], 30);

    let shown = text(&s, &["lookup", "describe", &id]);
    assert!(
        shown.contains("Search  Kessler & Vane - Night Signal (Extended Mix)"),
        "{shown}"
    );
}

/// The candidate a shop lists for the track: the same recording, spelled
/// the shop's way.
fn listing() -> Value {
    json!({
        "artist": "Vane, Kessler",
        "title": "Night Signal (Extended Mix)",
        "duration": "0:31",
        "label": "LOWFIELD",
        "release_date": "2019-04-12",
        "genre": ["Tech House"],
    })
}

fn check(sandbox: &Sandbox, id: &str, candidate: &Value, extra: &[&str]) -> Value {
    let candidate = candidate.to_string();
    let mut args = vec![
        "lookup",
        "check",
        id,
        "--candidate",
        &candidate,
        "--address",
        ADDRESS,
        "--date",
        LOOKED_UP,
    ];
    args.extend_from_slice(extra);
    ok(sandbox, &args)
}

/// Declares the two kinds a shop's listing has beyond the built-in ones.
fn declare_label_and_release_date(sandbox: &Sandbox) {
    std::fs::create_dir_all(sandbox.sidecar_dir()).unwrap();
    std::fs::write(
        sandbox.sidecar_dir().join("kinds.toml"),
        "[kinds.label]\nnature = \"judgement\"\nvalue = \"text\"\n\n\
         [kinds.release_date]\nnature = \"judgement\"\nvalue = \"text\"\n",
    )
    .unwrap();
}

/// Every file under `root` and its bytes, SQLite's shared-memory index left
/// out (any read through an open connection touches it).
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    if !root.exists() {
        return out;
    }
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.to_string_lossy().ends_with("-shm") {
                continue;
            }
            if path.is_dir() {
                out.insert(path.clone(), Vec::new());
                todo.push(path);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// The Library's folder, the audio files and the Sidecar, byte for byte.
fn everything(sandbox: &Sandbox) -> Vec<BTreeMap<PathBuf, Vec<u8>>> {
    vec![
        snapshot(&sandbox.path().join("library")),
        snapshot(&sandbox.path().join("audio")),
        snapshot(&sandbox.sidecar_dir()),
    ]
}

fn changed(
    before: &[BTreeMap<PathBuf, Vec<u8>>],
    after: &[BTreeMap<PathBuf, Vec<u8>>],
) -> Vec<PathBuf> {
    before
        .iter()
        .zip(after)
        .flat_map(|(before, after)| {
            before
                .keys()
                .chain(after.keys())
                .filter(|path| before.get(*path) != after.get(*path))
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn a_matching_candidate_is_accepted_and_its_fields_become_facts_with_the_address_and_date() {
    let s = Sandbox::new();
    let (id, _) = night_signal(&s);
    declare_label_and_release_date(&s);
    ok(&s, &["lookup", "describe", &id]);
    let library = snapshot(&s.path().join("library"));
    let mut watcher = s.client();
    watcher.subscribe().unwrap();

    let checked = check(&s, &id, &listing(), &[]);

    assert_eq!(checked["verdict"], "accepted", "{checked}");
    let facts = checked["facts"].as_array().unwrap();
    let recorded: BTreeMap<&str, &Value> = facts
        .iter()
        .map(|fact| (fact["kind"].as_str().unwrap(), &fact["value"]))
        .collect();
    assert_eq!(
        recorded,
        BTreeMap::from([
            ("artist", &json!("Vane, Kessler")),
            ("genre", &json!(["Tech House"])),
            ("label", &json!("LOWFIELD")),
            ("release_date", &json!("2019-04-12")),
            ("title", &json!("Night Signal (Extended Mix)")),
        ]),
        "every field but the duration, as the shop has it"
    );
    let run = checked["run"]["id"].as_i64().unwrap();
    for fact in facts {
        assert_eq!(fact["source"], json!({"kind": "web", "name": ADDRESS}));
        assert_eq!(fact["track_id"], json!(id));
        assert_eq!(fact["run"], run);
    }

    // As any client reads them: the Facts with the address as their
    // source, and their Run with the date.
    let shown = ok(&s, &["facts", "show", &id]);
    let from_the_shop = shown["facts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| fact["source"]["name"] == ADDRESS)
        .count();
    assert_eq!(from_the_shop, 5, "{shown}");
    let run_shown = ok(&s, &["runs", "show", &run.to_string()]);
    let description = run_shown["run"]["description"].as_str().unwrap();
    assert!(description.contains(ADDRESS), "{run_shown}");
    assert!(description.contains(LOOKED_UP), "{run_shown}");

    // The check is a Decision, answered, in the same Run, holding the
    // address and the date it can be asked again from.
    let decision = &checked["decision"];
    assert_eq!(decision["topic"], "lookup_match");
    assert_eq!(decision["track_id"], json!(id));
    assert_eq!(decision["run"], run);
    assert_eq!(decision["context"]["address"], ADDRESS);
    assert_eq!(decision["context"]["date"], LOOKED_UP);
    assert_eq!(decision["context"]["candidate"], listing());
    let answers = decision["answers"].as_array().unwrap();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0]["choice"], "accepted");
    assert_eq!(
        answers[0]["decider"],
        json!({"kind": "tool", "name": "lookup-check"})
    );
    let stored = ok(&s, &["decisions", "show", &decision["id"].to_string()]);
    assert_eq!(&stored, decision);

    // A second client hears of each Fact, then of the Decision; the
    // Library is not touched.
    let events =
        common::barrier::heard_exactly(&mut watcher, 6, std::time::Duration::from_millis(500));
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| {
            assert_eq!(event.event, "sidecar_changed", "{events:?}");
            event.data["kind"].as_str().unwrap()
        })
        .collect();
    assert_eq!(kinds.len(), 6);
    assert_eq!(kinds[5], "decision", "{events:?}");
    assert_eq!(events[5].data["decision_id"], decision["id"]);
    assert!(changed(&[library], &[snapshot(&s.path().join("library"))]).is_empty());
}

#[test]
fn another_mix_or_a_duration_outside_the_tolerance_is_held_and_makes_no_fact() {
    let s = Sandbox::new();
    let (id, _) = night_signal(&s);

    let mut radio = listing();
    radio["title"] = json!("Night Signal");
    radio["mix_name"] = json!("Radio Edit");
    let held = check(&s, &id, &radio, &[]);
    assert_eq!(held["verdict"], "held", "{held}");
    assert!(
        held["reason"]
            .as_str()
            .unwrap()
            .contains("the mix name is \"Radio Edit\", not \"Extended Mix\""),
        "{held}"
    );
    assert_eq!(held["facts"], json!([]));
    assert_eq!(held["decision"]["answers"][0]["choice"], "held");

    let mut long = listing();
    long["duration"] = json!(45);
    let held = check(&s, &id, &long, &[]);
    assert_eq!(held["verdict"], "held", "{held}");
    assert!(
        held["reason"].as_str().unwrap().contains("0:45, not 0:30"),
        "{held}"
    );
    // Within a wider tolerance, the same candidate is accepted.
    let wider = check(&s, &id, &long, &["--tolerance", "20"]);
    assert_eq!(wider["verdict"], "accepted", "{wider}");

    let shown = ok(&s, &["facts", "show", &id]);
    let looked_up = shown["facts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|fact| fact["source"]["kind"] == "web")
        .count();
    assert_eq!(
        looked_up, 3,
        "only the accepted check's artist, title and genre: {shown}"
    );
}

#[test]
fn a_clearly_different_candidate_is_rejected_with_the_reason() {
    let s = Sandbox::new();
    let (id, _) = night_signal(&s);
    let mut other = listing();
    other["title"] = json!("Day Signal");

    let rejected = check(&s, &id, &other, &[]);
    assert_eq!(rejected["verdict"], "rejected");
    assert_eq!(
        rejected["reason"],
        "the title is \"Day Signal\", not \"Night Signal\""
    );
    assert_eq!(rejected["facts"], json!([]));

    let other = other.to_string();
    let shown = text(
        &s,
        &[
            "lookup",
            "check",
            &id,
            "--candidate",
            &other,
            "--address",
            ADDRESS,
            "--date",
            LOOKED_UP,
        ],
    );
    assert!(
        shown.starts_with(&format!(
            "Rejected: the candidate from {ADDRESS} (looked up on {LOOKED_UP}) for track {id}.\n\
             Reason: the title is \"Day Signal\", not \"Night Signal\"."
        )),
        "{shown}"
    );
}

#[test]
fn each_check_is_recorded_as_a_decision_and_a_refused_one_records_nothing() {
    let s = Sandbox::new();
    let (id, _) = night_signal(&s);
    let mut radio = listing();
    radio["title"] = json!("Night Signal");
    radio["mix_name"] = json!("Radio Edit");
    let mut stranger = listing();
    stranger["artist"] = json!("Someone Else");
    for candidate in [listing(), radio, stranger] {
        check(&s, &id, &candidate, &[]);
    }
    let before = everything(&s);

    let untitled = json!({"artist": "Kessler & Vane"}).to_string();
    let error = refused(
        &s,
        &[
            "lookup",
            "check",
            &id,
            "--candidate",
            &untitled,
            "--address",
            ADDRESS,
            "--date",
            LOOKED_UP,
        ],
    );
    assert_eq!(error["code"], "invalid_candidate");
    let listed = listing().to_string();
    let error = refused(
        &s,
        &[
            "lookup",
            "check",
            &id,
            "--candidate",
            &listed,
            "--address",
            ADDRESS,
            "--date",
            "10 November",
        ],
    );
    assert_eq!(error["code"], "invalid_lookup");
    assert_eq!(changed(&before, &everything(&s)), Vec::<PathBuf>::new());

    let decisions = ok(
        &s,
        &[
            "decisions",
            "list",
            "--topic",
            "lookup_match",
            "--track",
            &id,
        ],
    );
    let choices: Vec<&str> = decisions["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|decision| decision["answers"][0]["choice"].as_str().unwrap())
        .collect();
    assert_eq!(choices, ["accepted", "held", "rejected"]);
}

#[test]
fn describing_and_a_dry_run_check_write_nothing_anywhere() {
    let s = Sandbox::new();
    let (id, _) = night_signal(&s);
    declare_label_and_release_date(&s);
    // Started, with no Sidecar store yet.
    ok(&s, &["tracks", "show", &id]);
    let before = everything(&s);
    assert!(!s.sidecar_dir().join("sidecar.db").exists());
    let mut watcher = s.client();
    watcher.subscribe().unwrap();

    ok(&s, &["lookup", "describe", &id]);
    let dry = check(&s, &id, &listing(), &["--dry-run"]);
    assert_eq!(dry["verdict"], "accepted");
    assert_eq!(dry["decision"], Value::Null);
    assert_eq!(dry["facts"], json!([]));
    let proposed: Vec<&str> = dry["proposed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| fact["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        proposed,
        ["artist", "genre", "label", "release_date", "title"]
    );

    assert_eq!(changed(&before, &everything(&s)), Vec::<PathBuf>::new());
    assert!(!s.sidecar_dir().join("sidecar.db").exists());
    assert!(
        watcher
            .next_event(std::time::Duration::from_millis(500))
            .unwrap()
            .is_none(),
        "a read tells no one anything"
    );
}

#[test]
fn a_missing_file_is_said_and_its_tags_are_not_guessed() {
    let s = Sandbox::new();
    let (id, file) = night_signal(&s);
    std::fs::remove_file(&file).unwrap();
    let described = ok(&s, &["lookup", "describe", &id]);
    assert_eq!(described["file"]["tags"], Value::Null);
    assert_eq!(described["file"]["problem"], "the audio file is not there");
    assert_eq!(described["title"], "Night Signal");
}

/// One version of a package in `Cargo.lock`: its source (none for this
/// repository's own crates) and the names of what it depends on.
type Locked = (Option<String>, Vec<String>);

/// The packages `Cargo.lock` says `root` is built from, itself included.
fn dependencies_of(root: &str) -> BTreeMap<String, Option<String>> {
    let lock =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
            .unwrap();
    // name -> (source, dependency names)
    let mut packages: BTreeMap<String, Vec<Locked>> = BTreeMap::new();
    for block in lock.split("[[package]]").skip(1) {
        let mut name = String::new();
        let mut source = None;
        let mut deps = Vec::new();
        let mut in_deps = false;
        for line in block.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("name = ") {
                name = value.trim_matches('"').to_owned();
            } else if let Some(value) = line.strip_prefix("source = ") {
                source = Some(value.trim_matches('"').to_owned());
            } else if line.starts_with("dependencies = [") {
                in_deps = true;
            } else if in_deps && line.starts_with(']') {
                in_deps = false;
            } else if in_deps {
                let dep = line.trim_matches(|c| c == '"' || c == ',');
                deps.push(dep.split(' ').next().unwrap().to_owned());
            }
        }
        packages.entry(name).or_default().push((source, deps));
    }
    let mut seen = BTreeMap::new();
    let mut todo = vec![root.to_owned()];
    while let Some(name) = todo.pop() {
        if seen.contains_key(&name) {
            continue;
        }
        let versions = packages.get(&name).cloned().unwrap_or_default();
        seen.insert(
            name,
            versions.first().and_then(|(source, _)| source.clone()),
        );
        for (_, deps) in versions {
            todo.extend(deps);
        }
    }
    seen
}

#[test]
fn no_command_in_wreckordbox_uses_the_network() {
    let built_from = dependencies_of("wreckordbox");
    assert!(
        built_from.contains_key("wreckordbox-service"),
        "{built_from:?}"
    );

    // No client of the web, of name lookups or of encrypted connections is
    // built into the program at all.
    const NETWORK_CRATES: &[&str] = &[
        "reqwest",
        "ureq",
        "hyper",
        "hyper-util",
        "h2",
        "http-body",
        "curl",
        "curl-sys",
        "isahc",
        "attohttpc",
        "surf",
        "rustls",
        "native-tls",
        "tokio-rustls",
        "tungstenite",
        "trust-dns-resolver",
        "hickory-resolver",
        "async-std",
    ];
    let reached: Vec<&String> = built_from
        .keys()
        .filter(|name| NETWORK_CRATES.contains(&name.as_str()))
        .collect();
    assert!(reached.is_empty(), "network crates built in: {reached:?}");

    // And none of this repository's own crates it is built from opens an
    // internet socket or runs a program that would fetch for it: its only
    // sockets are the Library service's local (Unix) ones.
    const NETWORK_CODE: &[&str] = &[
        "TcpStream",
        "TcpListener",
        "UdpSocket",
        "ToSocketAddrs",
        "SocketAddr",
        "IpAddr",
        "Domain::IPV4",
        "Domain::IPV6",
        "Command::new(\"curl\"",
        "Command::new(\"wget\"",
    ];
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut offending = Vec::new();
    for (name, source) in &built_from {
        if source.is_some() {
            continue; // from a registry, covered above
        }
        let src = crates.join(name).join("src");
        assert!(src.is_dir(), "{name} is not under crates/");
        let mut todo = vec![src];
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    todo.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for needle in NETWORK_CODE {
                        if text.contains(needle) {
                            offending.push(format!("{}: {needle}", path.display()));
                        }
                    }
                }
            }
        }
    }
    assert!(offending.is_empty(), "{offending:#?}");
}
