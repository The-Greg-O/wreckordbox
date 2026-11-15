//! A Library hosted on a Stick is read-only for everything the Sidecar
//! records (wbx-8yh; the DJ's decision of 2026-11-14, option 3): every
//! command that records in the Sidecar is refused with one named refusal,
//! `sidecar_on_stick`, before anything is written, with the Stick unlocked
//! or not; every read still answers; ordinary Library edits still work. A
//! Library on this computer is unaffected.
//!
//! A temporary folder carrying the stand-in marker is the Stick. The
//! separation engine is the fake one, and it is never asked.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use common::Sandbox;
use serde_json::{json, Value};

/// Every command that records in the Sidecar: refused on a Stick.
const RECORDS: &[&str] = &[
    "decisions.answer",
    "decisions.create",
    "decisions.decide",
    "facts.record",
    "intensity.measure",
    "mutants.clear",
    "mutants.create",
    "mutants.edit",
    "mutants.remove",
    "projection.acknowledge",
    "projection.apply",
    "projection.mytags.apply",
    "review.accept",
    "review.reject",
    "review.reopen",
    "runs.start",
    "schema.convert",
    "setplans.build",
    "stems.separate",
];

/// Ordinary Library edits (and the Runs they are attributed to): allowed.
const LIBRARY_EDITS: &[&str] = &[
    "intake.set_tags",
    "mytags.assign",
    "mytags.create",
    "mytags.create_category",
    "mytags.delete",
    "mytags.merge",
    "mytags.move",
    "mytags.rename",
    "mytags.reorder",
    "mytags.unassign",
    "playlists.copy",
    "playlists.delete",
    "playlists.remove",
    "playlists.reorder",
    "protection.set",
    "runs.finish",
    "tracks.rate",
    "tracks.remove_from_collection",
    "tracks.set_prep_colour",
];

/// Neither: Stick writers with their own held confirmation, a Stick's own
/// dry run (it reads a Stick, not the Sidecar), and the connection's own
/// controls.
const OTHERS: &[&str] = &[
    "intensity.cancel",
    "stems.write",
    "sticks.dry_run",
    "sticks.load",
    "status",
    "stop",
    "subscribe",
];

/// Every read, with the arguments it is asked with here.
fn reads(track: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("decisions.get", json!({ "id": 1 })),
        ("decisions.list", json!({})),
        ("facts.kinds", json!({})),
        ("facts.of_track", json!({ "id": track })),
        ("facts.standing", json!({ "id": track, "kind": "mood" })),
        ("history.list", json!({})),
        ("history.tracks", json!({ "id": "1" })),
        ("intake.colours", json!({})),
        ("intake.counts", json!({})),
        ("intake.list", json!({})),
        ("intake.problems", json!({ "label": "Red" })),
        ("intake.tag_panel", json!({ "id": track })),
        ("intensity.show", json!({ "id": track })),
        ("mutants.get", json!({ "id": 1 })),
        ("mutants.list", json!({})),
        ("mytags.list", json!({})),
        ("mytags.of_track", json!({ "id": track })),
        ("mytags.tracks", json!({ "id": "1" })),
        ("playlists.tracks", json!({ "id": "1" })),
        ("playlists.tree", json!({})),
        ("projection.barriers", json!({})),
        ("projection.mytags.plan", json!({})),
        ("projection.plan", json!({})),
        ("protection.get", json!({})),
        ("review.rejected", json!({})),
        ("review.waiting", json!({})),
        ("runs.active", json!({})),
        ("runs.list", json!({})),
        ("runs.show", json!({ "id": 1 })),
        ("schema.check", json!({})),
        (
            "setplans.check",
            json!({ "recipe": "version = 1\ntracks = 2\n" }),
        ),
        ("setplans.list", json!({})),
        ("setplans.show", json!({ "id": 1 })),
        ("stems.show", json!({ "id": track })),
        ("stems.status", json!({ "id": track })),
        ("sticks.list", json!({})),
        ("tracks.get", json!({ "id": track })),
        ("tracks.list", json!({})),
        ("tracks.query", json!({})),
    ]
}

/// Codes a read may answer with that still mean it answered (the thing
/// asked about is not there, or its arguments are this test's guesses);
/// never the Sidecar's or the Stick's refusal.
const ANSWERED_ERRORS: &[&str] = &[
    "decision_not_found",
    "history_not_found",
    "tag_schema_missing",
    "mutant_not_found",
    "run_not_found",
    "set_plan_not_found",
    "track_not_found",
    "playlist_not_found",
    "my_tag_not_found",
    "bad_request",
];

/// Every command name the service's sources declare: each `const` naming
/// a `group.command`, and the protocol's own commands.
fn declared_commands() -> BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../wreckordbox-service/src");
    let mut out = BTreeSet::new();
    let mut todo = vec![root];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                todo.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for line in text.lines() {
                let line = line.trim();
                let Some(rest) = line
                    .strip_prefix("pub const ")
                    .or_else(|| line.strip_prefix("pub(crate) const "))
                    .or_else(|| line.strip_prefix("const "))
                else {
                    continue;
                };
                let Some((_, value)) = rest.split_once(": &str = \"") else {
                    continue;
                };
                let Some(value) = value.strip_suffix("\";") else {
                    continue;
                };
                let protocol = path.ends_with("protocol.rs")
                    && matches!(value, "status" | "subscribe" | "stop");
                if protocol
                    || (value.contains('.')
                        && value
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.'))
                {
                    out.insert(value.to_owned());
                }
            }
        }
    }
    out
}

/// Every file of the folder and its bytes; SQLite's shared-memory index is
/// left out (any read through an open connection touches it).
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.insert(path.clone(), Vec::new());
                todo.push(path);
            } else if !path.to_string_lossy().ends_with("-shm") {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// A short 16-bit stereo 44.1 kHz WAV file of silence.
fn write_audio(path: &Path) {
    let frames: u32 = 44_100;
    let data_len = frames * 4;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&44_100_u32.to_le_bytes());
    out.extend_from_slice(&(44_100_u32 * 4).to_le_bytes());
    out.extend_from_slice(&4_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(out.len() + data_len as usize, 0);
    std::fs::write(path, out).unwrap();
}

fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.json()
}

/// A sandbox with one imported track and Facts already recorded on this
/// computer: one of the DJ's, an agent's Energy waiting in Review.
fn with_facts() -> (Sandbox, String) {
    let sandbox = Sandbox::new();
    let audio = sandbox.path().join("audio/Long Track.wav");
    std::fs::create_dir_all(audio.parent().unwrap()).unwrap();
    write_audio(&audio);
    let mut id = String::new();
    sandbox.edit(|writer| id = writer.import_file(&audio).unwrap());
    // Below this threshold an agent's Energy waits in Review.
    std::fs::create_dir_all(sandbox.sidecar_dir()).unwrap();
    std::fs::write(
        sandbox.sidecar_dir().join("projection-policy.toml"),
        "[energy]\napply = 0.8\n",
    )
    .unwrap();
    ok(
        &sandbox,
        &["facts", "record", &id, "mood", "Dark", "--source", "dj"],
    );
    ok(
        &sandbox,
        &[
            "facts",
            "record",
            &id,
            "energy",
            "4",
            "--source",
            "agent:tagger",
            "--confidence",
            "0.6",
        ],
    );
    (sandbox, id)
}

/// Makes `folder` stand in for a Stick and unlocks it by name, as the DJ
/// would: the state in which a write could otherwise go through.
fn an_unlocked_stick(sandbox: &Sandbox, folder: &Path) {
    std::fs::write(
        folder.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let name = folder.file_name().unwrap().to_str().unwrap().to_owned();
    let unlocked = sandbox.cli(&["stick", "unlock", folder.to_str().unwrap(), "--name", &name]);
    assert!(unlocked.status.success(), "{}", unlocked.stderr);
}

/// Arguments a recording command is asked with: refused before they are
/// looked at, so any will do, but these are the shapes the CLI sends.
fn record_args(command: &str, track: &str) -> Value {
    match command {
        "facts.record" => json!({
            "facts": [{ "track": track, "kind": "mood", "value": "Bright" }],
            "source": { "kind": "dj" },
        }),
        "stems.separate" | "intensity.measure" => json!({ "id": track, "tracks": [track] }),
        "review.accept" | "review.reject" | "review.reopen" => {
            json!({ "facts": [2], "source": { "kind": "dj" } })
        }
        "projection.apply" | "projection.mytags.apply" => json!({ "source": { "kind": "dj" } }),
        "projection.acknowledge" => json!({ "barrier": 1, "source": { "kind": "dj" } }),
        "runs.start" => json!({ "source": { "kind": "dj" }, "description": "tidy" }),
        _ => json!({ "id": 1, "entry": "1", "source": { "kind": "dj" } }),
    }
}

fn assert_refused(sandbox: &Sandbox, command: &str, track: &str) {
    let sidecar = sandbox.sidecar_dir();
    let before = snapshot(&sidecar);
    let library = std::fs::read(sandbox.master_db()).unwrap();
    let error = sandbox
        .client()
        .call::<Value>(command, record_args(command, track))
        .expect_err(command);
    assert_eq!(error.code, "sidecar_on_stick", "{command}: {error:?}");
    assert!(
        error.message.contains(&format!("`{command}`")),
        "{command}: the refusal names the command: {}",
        error.message
    );
    assert!(
        error.message.contains("Stick")
            && error.message.contains("Nothing was written")
            && error.message.contains("this computer"),
        "{command}: says why and what the DJ can do: {}",
        error.message
    );
    assert_eq!(
        snapshot(&sidecar),
        before,
        "{command}: not a byte of the Sidecar changed"
    );
    assert_eq!(
        std::fs::read(sandbox.master_db()).unwrap(),
        library,
        "{command}: nor of the Library"
    );
}

#[test]
fn every_service_command_is_classified_for_a_library_on_a_stick() {
    let declared = declared_commands();
    let reads: BTreeSet<&str> = reads("1").into_iter().map(|(name, _)| name).collect();
    for name in &declared {
        let places = [
            RECORDS.contains(&name.as_str()),
            LIBRARY_EDITS.contains(&name.as_str()),
            OTHERS.contains(&name.as_str()),
            reads.contains(name.as_str()),
        ];
        assert_eq!(
            places.iter().filter(|&&p| p).count(),
            1,
            "service command `{name}` must be in exactly one of RECORDS, LIBRARY_EDITS, OTHERS and the reads"
        );
    }
    for name in RECORDS
        .iter()
        .chain(LIBRARY_EDITS)
        .chain(OTHERS)
        .chain(&reads)
    {
        assert!(
            declared.contains(*name),
            "`{name}` is listed but not declared"
        );
    }
}

#[test]
fn every_sidecar_record_is_refused_on_an_unlocked_stick_and_every_read_still_answers() {
    let (sandbox, track) = with_facts();
    an_unlocked_stick(&sandbox, &sandbox.sidecar_dir());

    for command in RECORDS {
        assert_refused(&sandbox, command, &track);
    }
    assert_eq!(
        sandbox.engine_runs(),
        0,
        "the separation engine was never asked"
    );

    let sidecar = sandbox.sidecar_dir();
    let before = snapshot(&sidecar);
    for (command, args) in reads(&track) {
        match sandbox.client().call::<Value>(command, args) {
            Ok(_) => {}
            Err(error) => assert!(
                ANSWERED_ERRORS.contains(&error.code.as_str()),
                "{command} still answers: {error:?}"
            ),
        }
    }
    assert_eq!(snapshot(&sidecar), before, "reading wrote nothing");

    // What was recorded before is still read: the Facts and the waiting one.
    let shown = ok(&sandbox, &["facts", "show", &track]);
    let kinds: BTreeSet<&str> = shown["facts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| fact["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, BTreeSet::from(["energy", "mood"]), "{shown}");
    let waiting = ok(&sandbox, &["review", "waiting"]);
    assert_eq!(waiting["count"], 1, "{waiting}");
}

#[test]
fn every_sidecar_record_is_refused_on_a_locked_stick_too() {
    let (sandbox, track) = with_facts();
    let sidecar = sandbox.sidecar_dir();
    an_unlocked_stick(&sandbox, &sidecar);
    assert!(sandbox
        .cli(&["stick", "lock", sidecar.to_str().unwrap()])
        .status
        .success());
    for command in [
        "facts.record",
        "review.reject",
        "projection.apply",
        "mutants.create",
    ] {
        assert_refused(&sandbox, command, &track);
    }
}

#[test]
fn a_library_on_a_stick_with_its_sidecar_elsewhere_records_nothing_either() {
    let (sandbox, track) = with_facts();
    let library = sandbox.master_db().parent().unwrap().to_path_buf();
    an_unlocked_stick(&sandbox, &library);
    for command in ["facts.record", "review.reject", "projection.apply"] {
        assert_refused(&sandbox, command, &track);
    }
    // An ordinary edit of that Library is still made.
    ok(&sandbox, &["tracks", "rate", &track, "4"]);
    let shown = ok(&sandbox, &["tracks", "show", &track]);
    assert_eq!(shown["rating"], 4, "{shown}");
}

#[test]
fn ordinary_library_edits_still_work_with_the_sidecar_on_an_unlocked_stick() {
    let (sandbox, track) = with_facts();
    an_unlocked_stick(&sandbox, &sandbox.sidecar_dir());
    ok(&sandbox, &["tracks", "rate", &track, "5"]);
    let shown = ok(&sandbox, &["tracks", "show", &track]);
    assert_eq!(shown["rating"], 5, "{shown}");
    // Its Ledger entry is the edit's own record, kept with it.
    let runs = ok(&sandbox, &["runs", "list"]);
    assert!(
        runs.to_string().contains("tracks.rate"),
        "the edit is in the Ledger: {runs}"
    );
}

#[test]
fn a_library_on_this_computer_records_as_before() {
    let (sandbox, track) = with_facts();
    let recorded = ok(
        &sandbox,
        &[
            "facts", "record", &track, "mood", "Bright", "--source", "dj",
        ],
    );
    assert!(
        recorded["facts"][0]["value"].to_string().contains("Bright"),
        "{recorded}"
    );
    let waiting = ok(&sandbox, &["review", "waiting"]);
    assert_eq!(waiting["count"], 1, "{waiting}");
    let fact = waiting["waiting"][0]["fact"].to_string();
    let rejected = ok(&sandbox, &["review", "reject", &fact, "--source", "dj"]);
    assert_eq!(rejected["count"], 1, "{rejected}");
    assert_eq!(ok(&sandbox, &["review", "waiting"])["count"], 0);
}

/// The check is the Sidecar handle's, made when a write would be kept, not
/// only when the command was let in: an Intensity measuring let in for a
/// Library on this computer, held just before it records, keeps nothing
/// once the Sidecar's folder turns out to be on a Stick.
#[test]
fn a_command_let_in_earlier_cannot_write_once_its_sidecar_is_on_a_stick() {
    use crate::fact_producers_hold::{
        facts_in_sidecar, finished, gated, import_audio, spawn, Gate,
    };
    use crate::intensity::signals;

    let sandbox = Sandbox::new();
    let track = import_audio(&sandbox, "Loud.wav", &signals::loud_busy(3.0));
    let gate = Gate::new();
    let _service = gated(&sandbox, &gate);
    ok(
        &sandbox,
        &["facts", "record", &track, "mood", "Dark", "--source", "dj"],
    );

    gate.hold("producer-intensity");
    let measuring = spawn(&sandbox, &["intensity", "measure", &track]);
    gate.reached("at-producer-intensity");
    // Now, with the measuring's Sidecar handle long obtained, the Sidecar's
    // folder is a Stick (and an unlocked one).
    an_unlocked_stick(&sandbox, &sandbox.sidecar_dir());
    let before = snapshot(&sandbox.sidecar_dir());
    gate.go("producer-intensity");
    let (measured, answer) = finished(measuring);

    assert!(
        !measured || answer["measured"] == 0,
        "nothing was measured into the Sidecar: {answer}"
    );
    assert!(
        answer.to_string().contains("Nothing was written"),
        "the refusal is shown: {answer}"
    );
    assert_eq!(
        snapshot(&sandbox.sidecar_dir()),
        before,
        "not a byte of the Sidecar changed after the hold"
    );
    assert_eq!(facts_in_sidecar(&sandbox, &track, "intensity"), 0);
}

const TAG_SCHEMA: &str = r#"
version = 1

[genre]
terms = [
  { id = "house", name = "House" },
  { id = "techno", name = "Techno" },
]

[dimensions.situation]
terms = [
  { id = "peak", name = "Peak" },
  { id = "warm-up", name = "Warm-up" },
]

[deck]
categories = [{ dimension = "situation", label = "Situation" }]
"#;

/// Setting a tag of a dimension that is not on the deck is pure Fact
/// recording (a Judgements-only edit: the Library does not change), so it
/// is refused like every other recording command; a deck dimension's tag
/// is an ordinary MyTag edit of the Library and is still made.
#[test]
fn a_tag_that_is_only_a_fact_is_refused_and_a_deck_tag_is_still_set() {
    let (sandbox, track) = with_facts();
    std::fs::write(sandbox.sidecar_dir().join("tag-schema.toml"), TAG_SCHEMA).unwrap();
    an_unlocked_stick(&sandbox, &sandbox.sidecar_dir());

    let sidecar = sandbox.sidecar_dir();
    let before = snapshot(&sidecar);
    let library = std::fs::read(sandbox.master_db()).unwrap();
    // As the DJ, which is what records a DJ Fact.
    let run = sandbox.cli(&[
        "--json", "intake", "tag", &track, "genre", "techno", "--source", "dj",
    ]);
    assert!(
        !run.status.success(),
        "a Fact-only tag is refused: {}",
        run.stdout
    );
    let error = run.error_json()["error"].clone();
    assert_eq!(error["code"], "sidecar_on_stick", "{error}");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("Nothing was written") && message.contains("this computer"),
        "{message}"
    );
    assert_eq!(
        snapshot(&sidecar),
        before,
        "not a byte of the Sidecar changed"
    );
    assert_eq!(std::fs::read(sandbox.master_db()).unwrap(), library);
    let shown = ok(&sandbox, &["facts", "show", &track]);
    assert!(
        !shown.to_string().contains("techno"),
        "no genre Fact was recorded: {shown}"
    );

    // The deck dimension: the Library's MyTags change.
    ok(
        &sandbox,
        &[
            "intake",
            "tag",
            &track,
            "situation",
            "peak",
            "--source",
            "dj",
        ],
    );
    let tags = ok(&sandbox, &["mytags", "show", &track]);
    assert!(tags.to_string().contains("Peak"), "{tags}");
}

/// A DJ Fact derived from an ordinary Library edit (the DJ's rating) is
/// part of that edit's record: kept with it on an unlocked Stick.
#[test]
fn the_djs_fact_derived_from_a_rating_is_kept_with_the_edit() {
    let (sandbox, track) = with_facts();
    an_unlocked_stick(&sandbox, &sandbox.sidecar_dir());
    ok(&sandbox, &["tracks", "rate", &track, "3", "--source", "dj"]);
    assert_eq!(ok(&sandbox, &["tracks", "show", &track])["rating"], 3);
    assert_eq!(
        ok(&sandbox, &["facts", "standing", &track, "energy"])["standing"]["value"],
        3
    );
}

/// What an earlier service left (an intent no entry settled) and a
/// service starting on a Library whose Sidecar is on a Stick.
fn with_an_unsettled_intent() -> Sandbox {
    let sandbox = Sandbox::new();
    {
        let sidecar = wreckordbox_sidecar::Sidecar::open(&sandbox.sidecar_dir()).unwrap();
        let source = wreckordbox_sidecar::Source::new(wreckordbox_sidecar::SourceKind::Dj, "dj");
        sidecar
            .begin_intent(
                &wreckordbox_sidecar::ledger::Intent {
                    source: &source,
                    run: None,
                    description: "An edit an earlier service began",
                    command: None,
                    at: 1,
                },
                &[],
            )
            .unwrap();
    }
    sandbox
}

#[test]
fn a_service_starting_on_an_unlocked_stick_brings_its_ledger_up_to_date() {
    let sandbox = with_an_unsettled_intent();
    an_unlocked_stick(&sandbox, &sandbox.sidecar_dir());
    let runs = ok(&sandbox, &["runs", "list"]);
    assert!(
        runs.to_string()
            .contains("An edit an earlier service began"),
        "the intent was settled into an entry: {runs}"
    );
}

#[test]
fn a_service_starting_on_a_locked_stick_waits_to_bring_its_ledger_up_to_date() {
    let sandbox = with_an_unsettled_intent();
    let sidecar = sandbox.sidecar_dir();
    std::fs::write(
        sidecar.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let before = snapshot(&sidecar);
    // Starts the service; the Ledger is not brought up to date, and says so.
    let run = sandbox.cli(&["--json", "runs", "list"]);
    assert!(!run.status.success(), "{}", run.stdout);
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "stick_protected", "{error}");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unlock"),
        "{error}"
    );
    assert_eq!(snapshot(&sidecar), before, "nothing half-done on the Stick");

    // Unlocked, the next edit brings it up to date first.
    let name = sidecar.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(sandbox
        .cli(&[
            "stick",
            "unlock",
            sidecar.to_str().unwrap(),
            "--name",
            &name
        ])
        .status
        .success());
    ok(&sandbox, &["tracks", "rate", "10003", "2"]);
    let runs = ok(&sandbox, &["runs", "list"]);
    assert!(
        runs.to_string()
            .contains("An edit an earlier service began"),
        "{runs}"
    );
}

/// The Library on a locked Stick, its Sidecar kept on this computer: the
/// Ledger is not brought up to date at the start, and its queries say so
/// rather than answer without the unsettled work.
#[test]
fn a_service_starting_on_a_locked_stick_with_its_sidecar_elsewhere_says_its_ledger_waits() {
    let sandbox = with_an_unsettled_intent();
    let library = sandbox.master_db().parent().unwrap().to_path_buf();
    std::fs::write(
        library.join(wreckordbox_stick_protection::STAND_IN_MARKER),
        b"",
    )
    .unwrap();
    let run = sandbox.cli(&["--json", "runs", "list"]);
    assert!(
        !run.status.success(),
        "not an answer without the unsettled work: {}",
        run.stdout
    );
    let error = run.error_json();
    assert_eq!(error["error"]["code"], "stick_protected", "{error}");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unlock"),
        "{error}"
    );

    let name = library.file_name().unwrap().to_str().unwrap().to_owned();
    assert!(sandbox
        .cli(&[
            "stick",
            "unlock",
            library.to_str().unwrap(),
            "--name",
            &name
        ])
        .status
        .success());
    ok(&sandbox, &["tracks", "rate", "10003", "2"]);
    let runs = ok(&sandbox, &["runs", "list"]);
    assert!(
        runs.to_string()
            .contains("An edit an earlier service began"),
        "{runs}"
    );
}

/// Stops this sandbox's service and waits until it is gone.
fn stop_service(sandbox: &Sandbox) {
    let socket = sandbox.config().socket_path(sandbox.location());
    if let Ok(mut client) = wreckordbox_service::Client::connect(&socket) {
        let _ = client.stop();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while wreckordbox_service::Client::connect(&socket).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "the service did not stop"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn count(sandbox: &Sandbox, sql: &str) -> i64 {
    rusqlite::Connection::open_with_flags(
        sandbox.sidecar_dir().join("sidecar.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
    .query_row(sql, [], |row| row.get(0))
    .unwrap()
}

/// An asserting edit whose change could not be recorded (its Stick locked
/// between the Library write and the Ledger) keeps its own intent, even
/// when the Stick is unlocked again before the assertion is settled: after
/// the service ends there is still a record naming the track, never
/// nothing.
#[test]
fn an_asserting_edit_whose_change_went_unrecorded_keeps_its_intent_for_recovery() {
    use crate::fact_producers_hold::{gated, Gate};

    let (sandbox, track) = with_facts();
    stop_service(&sandbox);
    let sidecar = sandbox.sidecar_dir();
    an_unlocked_stick(&sandbox, &sidecar);
    let name = sidecar.file_name().unwrap().to_str().unwrap().to_owned();
    let gate = Gate::new();
    let service = gated(&sandbox, &gate);
    gate.hold("ledger-record");
    gate.hold("ledger-assert");

    // Not as the DJ: its assertion is empty, and only settles the intent.
    let mut client = sandbox.client();
    let rated = {
        let track = track.clone();
        std::thread::spawn(move || client.assert_rating(&track, 5))
    };
    gate.reached("at-ledger-record");
    assert!(sandbox
        .cli(&["stick", "lock", sidecar.to_str().unwrap()])
        .status
        .success());
    gate.go("ledger-record");
    gate.reached("at-ledger-assert");
    assert!(sandbox
        .cli(&[
            "stick",
            "unlock",
            sidecar.to_str().unwrap(),
            "--name",
            &name
        ])
        .status
        .success());
    gate.go("ledger-assert");
    let _ = rated.join().unwrap();
    // The service ends before any later edit could write what it kept in
    // memory.
    drop(service);

    let named = format!("%{track}%");
    let owed = |sandbox: &Sandbox| -> i64 {
        rusqlite::Connection::open_with_flags(
            sandbox.sidecar_dir().join("sidecar.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
        .query_row(
            "SELECT (SELECT COUNT(*) FROM ledger_intent)
                  + (SELECT COUNT(*) FROM ledger
                     WHERE label || ' ' || COALESCE(payload, '') || ' ' || COALESCE(command, '') LIKE ?1)",
            [&named],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert!(
        owed(&sandbox) > 0,
        "an intent or an entry naming the track survives the end of the service"
    );
    // The next service settles it into an entry naming the track; the
    // rating was made.
    ok(&sandbox, &["runs", "list"]);
    assert_eq!(ok(&sandbox, &["tracks", "show", &track])["rating"], 5);
    assert!(
        count(
            &sandbox,
            &format!(
                "SELECT COUNT(*) FROM ledger WHERE kind = 'unconfirmed' AND \
                 label || ' ' || COALESCE(payload, '') || ' ' || COALESCE(command, '') LIKE '%{track}%'"
            ),
        ) > 0,
        "an unconfirmed entry names the track"
    );
}
