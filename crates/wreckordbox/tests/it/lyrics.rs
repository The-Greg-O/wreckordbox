//! `wreckordbox lyrics` (wbx-f6v) against a Library service on a throwaway
//! Library and Sidecar: lyrics read from each format's own tags, lyrics an
//! agent hands in with their web address and date, an explicit
//! "instrumental", listing and full-text search, the Ledger, and that
//! reads and dry runs write nothing. Every lyric here is synthetic,
//! written for these tests; the audio is the repository's own synthetic
//! fixtures.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use common::barrier::{facts_announced, told_nothing};
use common::Sandbox;
use lofty::config::WriteOptions;
use lofty::id3::v2::{
    BinaryFrame, Frame, FrameId, Id3v2Tag, SyncTextContentType, SynchronizedTextFrame,
    TimestampFormat, UnsynchronizedTextFrame,
};
use lofty::mp4::{Atom, AtomData, AtomIdent, Ilst};
use lofty::ogg::VorbisComments;
use lofty::prelude::TagExt;
use lofty::TextEncoding;
use serde_json::{json, Value};

#[path = "../../../wreckordbox-sidecar/tests/fingerprint_support/mod.rs"]
mod fixture;

fn ok(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(run.status.success(), "{args:?}: {}", run.stderr);
    run.json()
}

fn refused(sandbox: &Sandbox, args: &[&str]) -> Value {
    let mut all = vec!["--json"];
    all.extend_from_slice(args);
    let run = sandbox.cli(&all);
    assert!(!run.status.success(), "{args:?} should fail: {}", run.stdout);
    run.error_json()["error"].clone()
}

fn with_input(sandbox: &Sandbox, args: &[&str], input: &str) -> Value {
    use std::io::Write as _;
    let mut child = sandbox
        .command()
        .arg("--json")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{args:?}: {stderr}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn id3_with(frames: Vec<Frame<'static>>) -> Id3v2Tag {
    let mut tag = Id3v2Tag::new();
    for frame in frames {
        tag.insert(frame);
    }
    tag
}

fn uslt(language: &[u8; 3], text: &str) -> Frame<'static> {
    Frame::UnsynchronizedText(UnsynchronizedTextFrame::new(
        TextEncoding::UTF8,
        *language,
        String::new(),
        text.to_owned(),
    ))
}

fn sylt(lines: &[(u32, &str)]) -> Frame<'static> {
    let frame = SynchronizedTextFrame::new(
        TextEncoding::UTF8,
        *b"eng",
        TimestampFormat::MS,
        SyncTextContentType::Lyrics,
        None,
        lines.iter().map(|(at, t)| (*at, (*t).to_owned())).collect(),
    );
    Frame::Binary(BinaryFrame::new(
        FrameId::new("SYLT").unwrap(),
        frame.as_bytes().unwrap(),
    ))
}

/// A synthetic file of `format` with lyrics written into its tags the way
/// that format keeps them; imported into the Library. Answers the track
/// and the file.
fn tagged(sandbox: &Sandbox, format: &str, name: &str) -> (String, PathBuf) {
    let dir = sandbox.path().join("audio").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let path = fixture::write(&dir, format);
    let options = WriteOptions::default();
    match name {
        "mp3-uslt" => id3_with(vec![uslt(b"eng", "Paper lanterns drift over the harbour\nwe sail before the dawn")])
            .save_to_path(&path, options)
            .unwrap(),
        "aiff-uslt" => id3_with(vec![uslt(b"XXX", "Copper wires hum beneath the city")])
            .save_to_path(&path, options)
            .unwrap(),
        "wav-sylt" => id3_with(vec![sylt(&[(1500, "Velvet thunder"), (4250, "rolls across the plain")])])
            .save_to_path(&path, options)
            .unwrap(),
        "flac-unsynced" => {
            let mut comments = VorbisComments::new();
            comments.insert("UNSYNCEDLYRICS".to_owned(), "Salt and static on the radio".to_owned());
            comments.save_to_path(&path, options).unwrap();
        }
        "m4a-lyr" => {
            let mut ilst = Ilst::new();
            ilst.insert(Atom::new(
                AtomIdent::Fourcc(*b"\xa9lyr"),
                AtomData::UTF8("Glass rivers carry the morning light".to_owned()),
            ));
            ilst.save_to_path(&path, options).unwrap();
        }
        _ => {}
    }
    let mut id = String::new();
    sandbox.edit(|writer| id = writer.import_file(&path).unwrap());
    (id, path)
}

/// Every file under `dir` and its bytes, `-shm` aside (a read through the
/// open writer touches SQLite's shared memory index).
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else if !path.to_string_lossy().ends_with("-shm") {
            out.insert(path.clone(), std::fs::read(&path).unwrap());
        }
    }
    out
}

#[test]
fn lyrics_in_each_formats_own_tags_are_imported_into_the_sidecar_and_the_files_are_only_read() {
    let sandbox = Sandbox::new();
    let (mp3, mp3_file) = tagged(&sandbox, "mp3-cbr", "mp3-uslt");
    let (aiff, _) = tagged(&sandbox, "aiff", "aiff-uslt");
    let (wav, _) = tagged(&sandbox, "wav", "wav-sylt");
    let (flac, _) = tagged(&sandbox, "flac", "flac-unsynced");
    let (m4a, _) = tagged(&sandbox, "m4a", "m4a-lyr");
    let (bare, _) = tagged(&sandbox, "wav", "bare");
    // A fixture track whose audio file is not there.
    let missing = "10001";
    let tracks: [&str; 7] = [&mp3, &aiff, &wav, &flac, &m4a, &bare, missing];
    let audio = files(&sandbox.path().join("audio"));
    let master = std::fs::read(sandbox.master_db()).unwrap();

    let mut args = vec!["lyrics", "import-embedded", "--dry-run", "--tracks"];
    args.extend(tracks);
    let dry = ok(&sandbox, &args);
    assert_eq!(dry["dry_run"], true);
    assert_eq!((dry["found"].as_u64(), dry["none"].as_u64(), dry["failed"].as_u64()), (Some(5), Some(1), Some(1)));
    assert_eq!(dry["run"], Value::Null);
    assert!(
        !sandbox.sidecar_dir().exists(),
        "a dry run makes no Sidecar"
    );

    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    let mut args = vec!["lyrics", "import-embedded", "--source", "agent:librarian", "--tracks"];
    args.extend(tracks);
    let imported = ok(&sandbox, &args);
    let by_track: BTreeMap<String, Value> = imported["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["track"]["id"].as_str().unwrap().to_owned(), t.clone()))
        .collect();
    let tag = |track: &str| by_track[track]["tag"].as_str().map(str::to_owned);
    assert_eq!(tag(&mp3).as_deref(), Some("ID3v2 USLT"));
    assert_eq!(tag(&aiff).as_deref(), Some("ID3v2 USLT"));
    assert_eq!(tag(&wav).as_deref(), Some("ID3v2 SYLT"));
    assert_eq!(tag(&flac).as_deref(), Some("Vorbis UNSYNCEDLYRICS"));
    assert_eq!(tag(&m4a).as_deref(), Some("MP4 ©lyr"));
    assert_eq!(by_track[&bare]["found"], "none");
    assert_eq!(by_track[&bare]["fact"], Value::Null);
    assert_eq!(by_track[missing]["found"], "failed");
    assert!(by_track[missing]["problem"].as_str().unwrap().contains("missing"));
    let recorded: Vec<i64> = by_track
        .values()
        .filter_map(|t| t["fact"].as_i64())
        .collect();
    assert_eq!(recorded.len(), 5);
    facts_announced(&mut watcher, &recorded);

    // Recorded in the caller's Run, every record its own Ledger entry.
    let run = &imported["run"];
    assert_eq!(run["source"], json!({"kind": "agent", "name": "librarian"}));
    let entries = ok(&sandbox, &["runs", "show", &run["id"].to_string()]);
    let records = entries["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "record" && e["store"] == "sidecar")
        .count();
    assert_eq!(records, 5, "{entries}");

    let mp3_shown = ok(&sandbox, &["lyrics", "show", &mp3]);
    assert_eq!(mp3_shown["status"], "lyrics");
    let value = &mp3_shown["standing"]["value"];
    assert_eq!(
        value["text"],
        "Paper lanterns drift over the harbour\nwe sail before the dawn"
    );
    assert_eq!(value["language"], "eng");
    assert_eq!(value["origin"], "embedded");
    assert_eq!(mp3_shown["standing"]["source"], json!({"kind": "tool", "name": "file-tags"}));
    assert_eq!(mp3_shown["standing"]["nature"], "judgement");
    assert!(mp3_shown["standing"]["fingerprint"].is_string());
    // XXX is ID3's "unknown language".
    assert_eq!(ok(&sandbox, &["lyrics", "show", &aiff])["standing"]["value"].get("language"), None);
    let synced = ok(&sandbox, &["lyrics", "show", &wav])["standing"]["value"].clone();
    assert_eq!(synced["synced"], true);
    assert_eq!(
        synced["lines"],
        json!([{"at_ms": 1500, "text": "Velvet thunder"}, {"at_ms": 4250, "text": "rolls across the plain"}])
    );
    assert_eq!(synced["text"], "Velvet thunder\nrolls across the plain");

    // Only read: no audio file and not the Library changed.
    assert_eq!(files(&sandbox.path().join("audio")), audio);
    assert_eq!(std::fs::read(&mp3_file).unwrap(), audio[&mp3_file]);
    assert_eq!(std::fs::read(sandbox.master_db()).unwrap(), master);
    let shown = ok(&sandbox, &["tracks", "show", &mp3]);
    assert!(!shown.to_string().contains("lanterns"), "the Library holds no lyrics: {shown}");
}

#[test]
fn an_agent_hands_lyrics_in_with_their_address_and_date_and_they_are_listed_and_searched() {
    let sandbox = Sandbox::new();
    let set = with_input(
        &sandbox,
        &[
            "lyrics", "set", "10002",
            "--source-url", "https://lyrics.example.test/songs/42",
            "--date", "2026-11-15",
            "--language", "en",
            "--source", "agent:matcher",
        ],
        "Neon tide pulls the harbour lights\nhold on through the night\n",
    );
    let fact = &set["fact"];
    assert_eq!(fact["source"], json!({"kind": "web", "name": "https://lyrics.example.test/songs/42"}));
    assert_eq!(set["run"]["source"], json!({"kind": "agent", "name": "matcher"}));
    assert_eq!(fact["value"]["address"], "https://lyrics.example.test/songs/42");
    assert_eq!(fact["value"]["retrieved"], "2026-11-15");
    assert_eq!(fact["value"]["origin"], "supplied");
    assert_eq!(fact["value"]["text"], "Neon tide pulls the harbour lights\nhold on through the night");

    // An explicit "no lyrics", so an agent stops looking.
    let instrumental = ok(&sandbox, &["lyrics", "set", "10003", "--instrumental", "--source", "dj"]);
    assert_eq!(instrumental["fact"]["value"]["instrumental"], true);
    assert_eq!(ok(&sandbox, &["lyrics", "show", "10003"])["status"], "instrumental");

    let listed = ok(&sandbox, &["lyrics", "list"]);
    let total = listed["total"].as_u64().unwrap();
    assert_eq!(listed["counts"]["lyrics"], 1);
    assert_eq!(listed["counts"]["instrumental"], 1);
    assert_eq!(listed["counts"]["unknown"].as_u64().unwrap(), total - 2);
    let unknown = ok(&sandbox, &["lyrics", "list", "--status", "unknown"]);
    assert!(unknown["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["status"] == "unknown" && t["id"] != "10002" && t["id"] != "10003"));
    let with = ok(&sandbox, &["lyrics", "list", "--status", "lyrics"]);
    assert_eq!(with["tracks"][0]["id"], "10002");
    assert_eq!(with["tracks"][0]["language"], "en");

    let found = ok(&sandbox, &["lyrics", "search", "harbour", "NIGHT"]);
    assert_eq!(found["engine"], "sqlite-fts5");
    let hits = found["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1, "{found}");
    assert_eq!(hits[0]["track"]["id"], "10002");
    assert_eq!(hits[0]["standing"], true);
    assert!(hits[0]["snippet"].as_str().unwrap().contains("[harbour]"));
    assert_eq!(ok(&sandbox, &["lyrics", "search", "harbour", "desert"])["hits"], json!([]));

    // The same source again replaces its words; the old ones are not found.
    with_input(
        &sandbox,
        &[
            "lyrics", "set", "10002",
            "--source-url", "https://lyrics.example.test/songs/42",
            "--date", "2026-11-16",
            "--source", "agent:matcher",
        ],
        "Quiet engines in the snow",
    );
    assert_eq!(ok(&sandbox, &["lyrics", "search", "harbour"])["hits"], json!([]));
    assert_eq!(ok(&sandbox, &["lyrics", "search", "engines"])["hits"][0]["track"]["id"], "10002");
    assert_eq!(ok(&sandbox, &["lyrics", "show", "10002"])["records"].as_array().unwrap().len(), 1);
}

#[test]
fn lyrics_that_do_not_hold_together_or_say_nobody_are_refused_and_nothing_is_written() {
    let sandbox = Sandbox::new();
    let mut client = {
        ok(&sandbox, &["status"]);
        sandbox.client()
    };
    let refuse = |client: &mut wreckordbox_service::Client, args: Value| {
        client
            .call::<Value>("lyrics.set", args)
            .unwrap_err()
            .code
    };
    let agent = json!({"kind": "agent", "name": "a"});
    assert_eq!(
        refuse(&mut client, json!({"id": "10002", "text": "words", "address": "https://x.test/1", "source": agent})),
        "invalid_lyrics"
    );
    assert_eq!(
        refuse(&mut client, json!({"id": "10002", "text": "words", "instrumental": true, "source": agent})),
        "invalid_lyrics"
    );
    assert_eq!(
        refuse(&mut client, json!({"id": "10002", "text": "   ", "source": agent})),
        "invalid_lyrics"
    );
    assert_eq!(refuse(&mut client, json!({"id": "10002", "text": "words"})), "invalid_source");
    assert_eq!(
        refuse(&mut client, json!({"id": "99999", "text": "words", "source": agent})),
        "track_not_found"
    );
    assert_eq!(
        refused(&sandbox, &["lyrics", "import-embedded", "--tracks", "10001"])["code"],
        "invalid_source"
    );
    assert!(!sandbox.sidecar_dir().exists(), "nothing refused made a Sidecar");
    // Lyrics are the lyrics commands' own: not a general Fact.
    assert_eq!(
        refused(&sandbox, &["facts", "record", "10002", "lyrics", "{}", "--source", "agent:a", "--confidence", "1"])["code"],
        "kind_reserved"
    );
}

#[test]
fn reading_lyrics_writes_nothing_and_tells_nobody() {
    let sandbox = Sandbox::new();
    // With no Sidecar at all, reads answer and make none.
    assert_eq!(ok(&sandbox, &["lyrics", "show", "10002"])["status"], "unknown");
    assert_eq!(ok(&sandbox, &["lyrics", "search", "anything"])["hits"], json!([]));
    ok(&sandbox, &["lyrics", "list"]);
    assert!(!sandbox.sidecar_dir().exists());

    with_input(
        &sandbox,
        &["lyrics", "set", "10002", "--source", "agent:a"],
        "Lanterns and ladders",
    );
    let before = files(&sandbox.sidecar_dir());
    let mut watcher = sandbox.client();
    watcher.subscribe().unwrap();
    ok(&sandbox, &["lyrics", "show", "10002"]);
    ok(&sandbox, &["lyrics", "search", "lanterns"]);
    ok(&sandbox, &["lyrics", "list"]);
    ok(&sandbox, &["lyrics", "import-embedded", "--dry-run", "--tracks", "10002"]);
    told_nothing(&mut watcher);
    assert_eq!(files(&sandbox.sidecar_dir()), before);
}
