//! Rebuilding the Library from a Stick (wbx-b1d): `wreckordbox library
//! rebuild-from-stick <STICK> --music <FOLDER> [--dry-run]`, the real binary
//! against a service on a throwaway Library.
//!
//! The Stick is a temporary folder exported by the export pipeline, as
//! rekordbox on another computer would have written it: its tracks name
//! that computer's Library, not this one, so only the music folder can say
//! which file each is. The music folder is another temporary folder of
//! silent WAV files. Nothing here touches a mounted Stick, the real Library
//! or the real music folder.

use crate::common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::Sandbox;
use serde_json::{json, Value};

/// A silent 8 kHz mono 16-bit WAV of `tenths` tenths of a second: its
/// length sets its size, which is how two files of one name differ.
fn write_wav(path: &Path, tenths: u32) {
    let rate: u32 = 8_000;
    let data_len = rate * 2 * tenths / 10;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(44 + data_len as usize, 0);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, out).unwrap();
}

/// One track of the Stick: its title (= file stem), its length, its
/// rating, colour, comment and the Stick's tag ids.
struct StickTrack {
    title: &'static str,
    tenths: u32,
    rating: u8,
    colour: u8,
    comment: &'static str,
    tags: &'static [u64],
}

const TRACKS: [StickTrack; 6] = [
    StickTrack {
        title: "Low Sun",
        tenths: 10,
        rating: 5,
        colour: 3,
        comment: "opener",
        tags: &[21, 11],
    },
    StickTrack {
        title: "Paper",
        tenths: 12,
        rating: 3,
        colour: 0,
        comment: "",
        tags: &[23, 31],
    },
    StickTrack {
        title: "Kiln",
        tenths: 14,
        rating: 4,
        colour: 6,
        comment: "",
        tags: &[],
    },
    StickTrack {
        title: "Odd",
        tenths: 16,
        rating: 2,
        colour: 0,
        comment: "",
        tags: &[],
    },
    StickTrack {
        title: "Gone",
        tenths: 18,
        rating: 1,
        colour: 0,
        comment: "",
        tags: &[],
    },
    StickTrack {
        title: "Echo",
        tenths: 20,
        rating: 2,
        colour: 5,
        comment: "",
        tags: &[22],
    },
];

/// The Stick's MyTags, its categories in rekordbox's slots: 1 "Situation"
/// (Peak, Warm-up, Dark), as this Library's first; 2 "Vibe" (Dark) where
/// this Library has "Mood"; 3 "Mood" (Calm) where it has "Sound". So slot
/// 2 must give up "Mood" before slot 3 can take it.
const TAGS: [(u64, u32, &str, u8, u64); 7] = [
    (2, 1, "Situation", 1, 0),
    (1, 2, "Vibe", 1, 0),
    (3, 3, "Mood", 1, 0),
    (11, 1, "Dark", 0, 1),
    (21, 1, "Peak", 0, 2),
    (22, 2, "Warm-up", 0, 2),
    (31, 1, "Calm", 0, 3),
];
const SITUATION_DARK: (u64, u32, &str, u8, u64) = (23, 3, "Dark", 0, 2);

struct Rig {
    sandbox: Sandbox,
    _dirs: tempfile::TempDir,
    stick: PathBuf,
    music: PathBuf,
    /// "Echo", already in the Library at its file in the music folder.
    echo: String,
}

impl Rig {
    fn new() -> Self {
        let mut sandbox = Sandbox::empty();
        let dirs = tempfile::tempdir().unwrap();
        let source = dirs.path().join("other-computer");
        let music = dirs.path().join("music");
        let stick = dirs.path().join("GIG");
        std::fs::create_dir_all(&stick).unwrap();
        std::fs::write(
            stick.join(wreckordbox_stick_protection::STAND_IN_MARKER),
            b"",
        )
        .unwrap();

        let mut tracks = Vec::new();
        for (at, t) in TRACKS.iter().enumerate() {
            let file = source.join(format!("{}.wav", t.title));
            write_wav(&file, t.tenths);
            tracks.push(rbl_export::SourceTrack {
                id: 500 + at as u64,
                source_path: file,
                title: t.title.into(),
                artist: "Somebody".into(),
                rating: t.rating,
                color_id: t.colour,
                comment: t.comment.into(),
                bpm_x100: 12_000 + at as u32,
                my_tags: t.tags.to_vec(),
                ..rbl_export::SourceTrack::default()
            });
        }
        let my_tags: Vec<rbl_export::SourceMyTag> = TAGS
            .iter()
            .chain(std::iter::once(&SITUATION_DARK))
            .map(
                |&(id, seq, name, attribute, parent)| rbl_export::SourceMyTag {
                    id,
                    seq,
                    name: name.into(),
                    attribute,
                    parent,
                },
            )
            .collect();
        let playlists = vec![
            rbl_export::SourcePlaylist {
                id: 10,
                name: "Sets".into(),
                folder: true,
                ..Default::default()
            },
            rbl_export::SourcePlaylist {
                id: 11,
                name: "Friday".into(),
                parent_id: 10,
                track_indices: vec![0, 1, 2],
                ..Default::default()
            },
            rbl_export::SourcePlaylist {
                id: 12,
                name: "Warm Up".into(),
                track_indices: vec![5, 3, 0],
                ..Default::default()
            },
        ];
        rbl_export::export_full(
            &stick,
            &tracks,
            &playlists,
            &my_tags,
            None,
            None,
            &mut |_| {},
        )
        .unwrap();

        // The music folder: the same files, except that "Kiln" is there
        // twice at one size, "Odd" only at another size and "Gone" not at all.
        for t in &TRACKS {
            match t.title {
                "Gone" => {}
                "Odd" => write_wav(&music.join("Odd.wav"), t.tenths + 3),
                "Kiln" => {
                    write_wav(&music.join("a/Kiln.wav"), t.tenths);
                    write_wav(&music.join("b/kiln.WAV"), t.tenths);
                }
                "Low Sun" => write_wav(&music.join("Albums/Low Sun.wav"), t.tenths),
                _ => write_wav(&music.join(format!("{}.wav", t.title)), t.tenths),
            }
        }
        // rekordbox's four slots: Situation, Mood, Sound, Sub-genre.
        sandbox.four_categories();
        let mut echo = String::new();
        let echo_file = music.join("Echo.wav");
        sandbox.edit(|writer| {
            echo = writer.import_file(&echo_file).unwrap();
            writer.set_rating(&echo, 1).unwrap();
            writer.set_color(&echo, Some("5")).unwrap();
        });
        sandbox.sticks = vec![stick.clone()];
        // The service up, so a second client can reach it.
        assert!(sandbox.cli(&["status"]).status.success());
        Self {
            sandbox,
            _dirs: dirs,
            stick,
            music,
            echo,
        }
    }

    fn rebuild(&self, extra: &[&str]) -> common::Run {
        let mut args = vec![
            "--json",
            "library",
            "rebuild-from-stick",
            self.stick.to_str().unwrap(),
            "--music",
            self.music.to_str().unwrap(),
        ];
        args.extend_from_slice(extra);
        self.sandbox.cli(&args)
    }

    fn rebuilt(&self, extra: &[&str]) -> Value {
        let run = self.rebuild(extra);
        assert!(run.status.success(), "{extra:?}: {}", run.stderr);
        run.json()
    }

    fn call(&self, command: &str, args: Value) -> Value {
        self.sandbox.client().call::<Value>(command, args).unwrap()
    }

    /// The tree as `(depth, name, kind)`.
    fn tree(&self) -> Vec<(u64, String, String)> {
        self.call("playlists.tree", json!({}))["playlists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["depth"].as_u64().unwrap(),
                    p["name"].as_str().unwrap().to_owned(),
                    p["kind"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    fn playlist_id(&self, name: &str) -> String {
        let tree = self.call("playlists.tree", json!({}));
        tree["playlists"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap_or_else(|| panic!("no playlist {name}: {tree}"))["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn titles_in(&self, name: &str) -> Vec<String> {
        let id = self.playlist_id(name);
        self.call("playlists.tracks", json!({"id": id}))["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["title"].as_str().unwrap().to_owned())
            .collect()
    }

    /// Every track of the collection by title.
    fn collection(&self) -> BTreeMap<String, Value> {
        let list = self.call("tracks.list", json!({"limit": 100}));
        list["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let id = t["id"].as_str().unwrap();
                let full = self.call("tracks.get", json!({"id": id}));
                (full["title"].as_str().unwrap().to_owned(), full)
            })
            .collect()
    }

    fn runs(&self) -> Vec<Value> {
        self.call("runs.list", json!({}))["runs"]
            .as_array()
            .unwrap()
            .clone()
    }
}

/// Every file under `root` with its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.insert(path.clone(), Vec::new());
                stack.push(path);
            } else {
                out.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    out
}

/// A track's MyTags as `(category, tag)`, sorted.
fn tags_of(track: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = track["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["category"].as_str().unwrap().to_owned(),
                t["tag"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    out.sort();
    out
}

fn titles(list: &Value) -> Vec<String> {
    let mut out: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap().to_owned())
        .collect();
    out.sort();
    out
}

fn value_of<'a>(track: &'a Value, kind: &str) -> &'a Value {
    track["values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["value"] == kind)
        .unwrap_or_else(|| panic!("no {kind} in {track}"))
}

#[test]
fn a_dry_run_lists_what_would_be_carried_and_writes_nothing_anywhere() {
    let rig = Rig::new();
    // The service is up and its Sidecar made before the baseline.
    let runs_before = rig.runs();
    let library = rig.sandbox.master_db().parent().unwrap().to_path_buf();
    let library_before = snapshot(&library);
    let sidecar_before = snapshot(&rig.sandbox.sidecar_dir());
    let stick_before = snapshot(&rig.stick);
    let music_before = snapshot(&rig.music);

    let dry = rig.rebuilt(&["--dry-run"]);

    assert_eq!(dry["dry_run"], true);
    assert_eq!(titles(&dry["matched"]), ["Echo", "Low Sun", "Paper"]);
    let unmatched: BTreeMap<String, &Value> = dry["unmatched"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["title"].as_str().unwrap().to_owned(), t))
        .collect();
    assert_eq!(
        unmatched.keys().collect::<Vec<_>>(),
        ["Gone", "Kiln", "Odd"]
    );
    assert_eq!(unmatched["Gone"]["reason"], "no_file");
    assert_eq!(unmatched["Kiln"]["reason"], "ambiguous");
    assert_eq!(unmatched["Kiln"]["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(unmatched["Odd"]["reason"], "different_size");

    let low_sun = dry["matched"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["title"] == "Low Sun")
        .unwrap();
    assert_eq!(
        low_sun["file"],
        rig.music.join("Albums/Low Sun.wav").to_str().unwrap()
    );
    assert_eq!(low_sun["existing"], Value::Null);
    assert_eq!(value_of(low_sun, "rating")["to"], 5);
    assert_eq!(value_of(low_sun, "rating")["outcome"], "set");
    let echo = dry["matched"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["title"] == "Echo")
        .unwrap();
    assert_eq!(echo["existing"], rig.echo.as_str());
    assert_eq!(value_of(echo, "rating")["outcome"], "replaced");
    assert_eq!(value_of(echo, "rating")["from"], 1);
    assert_eq!(value_of(echo, "colour")["outcome"], "kept");

    let playlists: Vec<(String, String)> = dry["playlists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["path"].as_str().unwrap().to_owned(),
                p["action"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        playlists,
        [
            ("Sets".to_owned(), "create".to_owned()),
            ("Sets/Friday".to_owned(), "create".to_owned()),
            ("Warm Up".to_owned(), "create".to_owned()),
        ]
    );
    let friday = &dry["playlists"][1];
    assert_eq!(friday["tracks"], 2);
    assert_eq!(friday["left_out"], json!(["Kiln"]));

    let categories: BTreeMap<String, &Value> = dry["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["name"].as_str().unwrap().to_owned(), c))
        .collect();
    assert!(categories["Situation"]["library_category"].is_string());
    let actions: Vec<(String, String)> = categories["Situation"]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_owned(),
                t["action"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        actions,
        [
            ("Peak".to_owned(), "exists".to_owned()),
            ("Warm-up".to_owned(), "exists".to_owned()),
            ("Dark".to_owned(), "create".to_owned()),
        ]
    );
    // By slot: each Stick category takes the Library's category in its
    // place, renamed when the names differ.
    let slots: Vec<(String, u64, String, bool)> = dry["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_owned(),
                c["slot"].as_u64().unwrap(),
                c["library_name"].as_str().unwrap().to_owned(),
                c["rename"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        slots,
        [
            ("Situation".to_owned(), 1, "Situation".to_owned(), false),
            ("Vibe".to_owned(), 2, "Mood".to_owned(), true),
            ("Mood".to_owned(), 3, "Sound".to_owned(), true),
        ]
    );
    assert_eq!(categories["Vibe"]["tags"][0]["action"], "create");
    assert_eq!(categories["Mood"]["tags"][0]["name"], "Calm");
    let not_carried = dry["not_carried"].to_string();
    assert!(!not_carried.contains("MyTag"), "{not_carried}");
    for word in ["cues", "grids", "history"] {
        assert!(not_carried.contains(word), "{word} in {not_carried}");
    }
    let carried = &dry["values"];
    assert_eq!(carried["rating"], 3);
    assert_eq!(carried["my_tags"], 3);

    assert_eq!(rig.runs(), runs_before, "a dry run takes no Run");
    assert_eq!(
        snapshot(&library),
        library_before,
        "the Library is unchanged"
    );
    assert_eq!(
        snapshot(&rig.sandbox.sidecar_dir()),
        sidecar_before,
        "the Sidecar is unchanged"
    );
    assert_eq!(snapshot(&rig.stick), stick_before, "the Stick is unchanged");
    assert_eq!(
        snapshot(&rig.music),
        music_before,
        "the music folder is unchanged"
    );
    assert_eq!(rig.collection().len(), 1);
}

#[test]
fn applying_links_the_tracks_carries_their_values_and_rebuilds_the_playlists_in_one_import_run() {
    let rig = Rig::new();
    let runs_before = rig.runs().len();
    let mut watcher = rig.sandbox.client();
    watcher.subscribe().unwrap();

    let applied = rig.rebuilt(&["--source", "agent:rebuilder"]);
    assert_eq!(applied["dry_run"], false);
    assert!(applied["generation"].is_u64(), "{applied}");

    let collection = rig.collection();
    assert_eq!(
        collection.keys().collect::<Vec<_>>(),
        ["Echo", "Low Sun", "Paper"],
        "only matched tracks, Echo once"
    );
    let low_sun = &collection["Low Sun"];
    assert_eq!(
        low_sun["location"],
        rig.music.join("Albums/Low Sun.wav").to_str().unwrap(),
        "linked, not copied"
    );
    assert_eq!(low_sun["rating"], 5);
    assert_eq!(low_sun["colour_id"], 3);
    assert_eq!(low_sun["comment"], "opener");
    assert_eq!(low_sun["bpm"], 120.0);
    assert_eq!(
        tags_of(low_sun),
        [("Situation", "Peak"), ("Vibe", "Dark")].map(|(c, t)| (c.to_owned(), t.to_owned()))
    );
    assert_eq!(
        tags_of(&collection["Paper"]),
        [("Mood", "Calm"), ("Situation", "Dark")].map(|(c, t)| (c.to_owned(), t.to_owned())),
        "made in their categories"
    );
    let names: Vec<String> = rig.call("mytags.list", json!({}))["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        names,
        ["Situation", "Vibe", "Mood", "Sub-genre"],
        "renamed by slot"
    );
    let echo = &collection["Echo"];
    assert_eq!(echo["id"], rig.echo.as_str());
    assert_eq!(
        echo["rating"], 2,
        "the Stick's rating replaces the Library's"
    );
    let echo_tags: Vec<&str> = echo["my_tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["tag"].as_str().unwrap())
        .collect();
    assert_eq!(echo_tags, ["Warm-up"]);

    assert_eq!(
        rig.tree(),
        [
            (1, "Sets".to_owned(), "folder".to_owned()),
            (2, "Friday".to_owned(), "playlist".to_owned()),
            (1, "Warm Up".to_owned(), "playlist".to_owned()),
        ]
    );
    assert_eq!(rig.titles_in("Friday"), ["Low Sun", "Paper"]);
    assert_eq!(rig.titles_in("Warm Up"), ["Echo", "Low Sun"]);

    // One Run, an import, under the caller's source.
    let runs = rig.runs();
    assert_eq!(runs.len(), runs_before + 1, "{runs:#?}");
    assert_eq!(
        runs[0]["source"],
        json!({"kind": "agent", "name": "rebuilder"})
    );
    assert!(runs[0]["changes"].as_u64().unwrap() > 0, "{:#?}", runs[0]);

    // One change for the whole rebuild.
    let first = watcher
        .next_event(Duration::from_secs(5))
        .unwrap()
        .expect("a change");
    assert_eq!(first.event, "library_changed");
    assert!(
        watcher
            .next_event(Duration::from_millis(300))
            .unwrap()
            .is_none(),
        "a single coalesced change"
    );

    // A second rebuild duplicates nothing.
    let again = rig.rebuilt(&[]);
    let actions: Vec<&str> = again["playlists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["reuse", "replace", "replace"]);
    assert_eq!(rig.collection().len(), 3);
    assert_eq!(rig.tree().len(), 3);
    assert_eq!(rig.titles_in("Friday"), ["Low Sun", "Paper"]);
}

#[test]
fn under_library_protection_applying_is_refused_and_writes_nothing() {
    let rig = Rig::new();
    assert!(rig.sandbox.cli(&["protection", "on"]).status.success());
    let runs_before = rig.runs();
    let library = rig.sandbox.master_db().parent().unwrap().to_path_buf();
    let library_before = snapshot(&library);

    let run = rig.rebuild(&[]);
    assert!(!run.status.success());
    assert_eq!(
        run.error_json()["error"]["code"],
        "library_protected",
        "{}",
        run.stderr
    );

    assert_eq!(snapshot(&library), library_before);
    assert_eq!(rig.runs(), runs_before);
    assert_eq!(rig.collection().len(), 1);
    assert!(rig.tree().is_empty());
    // The dry run is a read, and still answers.
    assert_eq!(
        rig.rebuilt(&["--dry-run"])["matched"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn the_stick_is_only_read_locked_or_unlocked() {
    let rig = Rig::new();
    let before = snapshot(&rig.stick);
    rig.rebuilt(&["--dry-run"]);
    rig.rebuilt(&[]);
    assert_eq!(
        snapshot(&rig.stick),
        before,
        "locked: byte for byte as it was"
    );

    // The test plays the DJ, unlocking the Stick by name.
    let run = rig.sandbox.cli(&[
        "stick",
        "unlock",
        rig.stick.to_str().unwrap(),
        "--name",
        "GIG",
    ]);
    assert!(run.status.success(), "{}", run.stderr);
    let unlocked = snapshot(&rig.stick);
    rig.rebuilt(&["--dry-run"]);
    rig.rebuilt(&[]);
    assert_eq!(
        snapshot(&rig.stick),
        unlocked,
        "unlocked: byte for byte as it was"
    );
    assert_eq!(unlocked, before, "unlocking wrote nothing to it either");
}

#[test]
fn a_folder_that_is_not_a_stick_is_reported_and_nothing_is_written() {
    let rig = Rig::new();
    let empty = rig.sandbox.path().join("not-a-stick");
    std::fs::create_dir_all(&empty).unwrap();
    let run = rig.sandbox.cli(&[
        "--json",
        "library",
        "rebuild-from-stick",
        empty.to_str().unwrap(),
        "--music",
        rig.music.to_str().unwrap(),
    ]);
    assert!(!run.status.success());
    assert_eq!(
        run.error_json()["error"]["code"],
        "stick_unreadable",
        "{}",
        run.stderr
    );
    assert_eq!(rig.collection().len(), 1);
}
