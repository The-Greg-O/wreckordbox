//! The Intake and Problems queues, asked of a service in the test's own
//! process through its command interface. The Library is a throwaway
//! fixture.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use rbl_db::fixture::{self, Shape};
use rbl_db::write::Writer;
use rbl_db::LibraryLocation;
use serde_json::{json, Value};
use wreckordbox_service::{Config, Service};

/// A fixture Library a test shapes before a service opens it.
struct Fixture {
    dir: tempfile::TempDir,
    location: LibraryLocation,
}

impl Fixture {
    fn new(tracks: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        std::fs::create_dir_all(&library).unwrap();
        let location = fixture::build(
            &library,
            Shape {
                tracks,
                ..Shape::default()
            },
        )
        .unwrap();
        Self { dir, location }
    }

    /// Changes the Library with its own writer, before the service starts.
    fn edit(&self, change: impl FnOnce(&mut Writer)) {
        let mut writer =
            Writer::open(self.location.clone(), self.dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        change(&mut writer);
    }

    /// Gives the `index`th track the colour label numbered `colour`.
    fn colour(&self, index: usize, colour: u8) {
        self.edit(|writer| {
            writer
                .set_color(&fixture::track_id(index), Some(&colour.to_string()))
                .unwrap();
        });
    }

    fn start(self) -> Hosted {
        let config = Config {
            runtime_dir: self.dir.path().join("run"),
            idle_exit: None,
            recovery_dir: self.dir.path().join("recovery"),
        };
        // As on the machine of a DJ who edits: Library Protection is off.
        wreckordbox_service::protection::store(&config.recovery_dir, false).unwrap();
        let service = Service::start(self.location.clone(), &config).unwrap();
        Hosted {
            _dir: self.dir,
            service,
        }
    }
}

struct Hosted {
    _dir: tempfile::TempDir,
    service: Service,
}

impl Hosted {
    fn call(&self, command: &str, args: Value) -> Value {
        self.service
            .call(command, args)
            .unwrap_or_else(|e| panic!("{command} failed: {} {}", e.code, e.message))
    }
}

fn ids(page: &Value) -> Vec<&str> {
    page["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["id"].as_str().unwrap())
        .collect()
}

#[test]
fn intake_lists_the_tracks_with_no_prep_colour() {
    let fixture = Fixture::new(4);
    fixture.colour(1, 5);
    let hosted = fixture.start();

    let page = hosted.call("intake.list", json!({}));

    assert_eq!(ids(&page), ["10000", "10002", "10003"]);
    assert_eq!(page["total"], 3);
}

#[test]
fn a_queue_is_paged_like_the_track_query() {
    let hosted = Fixture::new(5).start();

    let first = hosted.call("intake.list", json!({ "limit": 2 }));
    assert_eq!(ids(&first), ["10000", "10001"]);
    assert_eq!(
        (
            &first["total"],
            &first["offset"],
            &first["limit"],
            &first["next_offset"]
        ),
        (&json!(5), &json!(0), &json!(2), &json!(2))
    );

    let last = hosted.call("intake.list", json!({ "limit": 2, "offset": 4 }));
    assert_eq!(ids(&last), ["10004"]);
    assert_eq!(last["next_offset"], Value::Null);

    let past = hosted.call("intake.list", json!({ "offset": 9 }));
    assert_eq!(ids(&past), Vec::<&str>::new());
    assert_eq!(past["total"], 5);
}

/// Names the colour label numbered `colour` as the DJ names one in
/// rekordbox's preferences. The fixture's colour table starts empty.
fn name_colour(writer: &mut Writer, colour: u8, name: &str) {
    writer
        .library()
        .connection()
        .execute(
            "INSERT INTO djmdColor (ID, ColorCode, SortKey, Commnt, UUID, created_at, updated_at)
             VALUES (?1, 0, ?1, ?2, 'fixture-colour-' || ?1, '2026-01-01', '2026-01-01')",
            (colour.to_string(), name),
        )
        .unwrap();
}

/// The DJ's own Prep colours, on the colours the DJ chose for them.
fn dj_labels(fixture: &Fixture) {
    fixture.edit(|writer| {
        for (colour, name) in [
            (1, "Buy"),
            (2, "Problem"),
            (3, "Orange"),
            (4, "Sample"),
            (5, "Clean"),
            (6, "Aqua"),
            (7, "Blue"),
            (8, "Delete"),
        ] {
            name_colour(writer, colour, name);
        }
    });
}

#[test]
fn problems_lists_the_tracks_whose_prep_colour_the_dj_named_problem() {
    let fixture = Fixture::new(4);
    dj_labels(&fixture);
    fixture.colour(0, 2);
    fixture.colour(1, 5);
    fixture.colour(3, 2);
    let hosted = fixture.start();

    let page = hosted.call("intake.problems", json!({}));

    assert_eq!(ids(&page), ["10000", "10003"]);
    assert_eq!(page["total"], 2);
    assert_eq!(
        page["prep_colour"],
        json!({ "id": 2, "colour": "Red", "label": "Problem" })
    );
}

#[test]
fn problems_says_so_when_no_colour_label_is_named_problem() {
    let fixture = Fixture::new(2);
    fixture.edit(|writer| {
        name_colour(writer, 2, "Red");
        name_colour(writer, 5, "Clean");
    });
    fixture.colour(0, 2);
    let hosted = fixture.start();

    let refused = hosted
        .service
        .call("intake.problems", json!({}))
        .unwrap_err();

    assert_eq!(refused.code, "problem_colour_not_found");
    assert!(
        refused.message.contains("\"Problem\""),
        "{}",
        refused.message
    );
    assert!(
        refused.message.contains("Clean (Green)"),
        "{}",
        refused.message
    );
}

#[test]
fn the_name_that_means_problem_can_be_given_and_is_matched_in_any_case() {
    let fixture = Fixture::new(3);
    fixture.edit(|writer| name_colour(writer, 7, "Kaputt"));
    fixture.colour(2, 7);
    let hosted = fixture.start();

    let page = hosted.call("intake.problems", json!({ "label": "kaputt" }));

    assert_eq!(ids(&page), ["10002"]);
    assert_eq!(page["prep_colour"]["label"], "Kaputt");
}

#[test]
fn counts_say_how_many_wait_in_each_queue_without_listing_them() {
    let fixture = Fixture::new(6);
    dj_labels(&fixture);
    fixture.colour(0, 2);
    fixture.colour(1, 5);
    fixture.colour(2, 2);
    let hosted = fixture.start();

    let counts = hosted.call("intake.counts", json!({}));

    assert_eq!(counts["intake"], 3);
    assert_eq!(counts["problems"], 2);
    assert_eq!(counts["problem_colour"]["label"], "Problem");
    assert!(counts.get("tracks").is_none());
}

#[test]
fn counts_still_count_intake_when_nothing_is_named_problem() {
    let hosted = Fixture::new(4).start();

    let counts = hosted.call("intake.counts", json!({}));

    assert_eq!(counts["intake"], 4);
    assert_eq!(counts["problems"], Value::Null);
    assert_eq!(
        counts["problems_unavailable"]["code"],
        "problem_colour_not_found"
    );
}

fn track<'a>(page: &'a Value, id: &str) -> &'a Value {
    page["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|track| track["id"] == id)
        .unwrap_or_else(|| panic!("track {id} is not on the page: {page}"))
}

#[test]
fn each_queued_track_says_what_it_still_needs() {
    let fixture = Fixture::new(3);
    let done = fixture::track_id(1);
    let fresh = fixture::track_id(2);
    fixture.edit(|writer| {
        // Everything the Library can show as done.
        writer.set_rating(&done, 4).unwrap();
        writer
            .set_my_tags(&done, &[fixture::MY_TAG_PEAK.to_owned()])
            .unwrap();
        writer.add_cue(&done, 1, 12_000).unwrap();
        writer.set_analysis_lock(&done, true).unwrap();
        // As rekordbox leaves a track it has only imported.
        writer
            .library()
            .connection()
            .execute(
                "UPDATE djmdContent SET Analysed = NULL WHERE ID = ?1",
                [&fresh],
            )
            .unwrap();
    });
    let hosted = fixture.start();

    let page = hosted.call("intake.list", json!({}));

    let untouched = track(&page, "10000");
    assert_eq!(
        untouched["needs"],
        json!(["energy", "tags", "hot_cues", "grid"])
    );
    assert_eq!(
        untouched["empty_dimensions"],
        json!([{ "id": "700001", "name": "Situation", "kind": "mytag_category" }])
    );
    assert_eq!(untouched["grid"], "not_confirmed");

    let done = track(&page, "10001");
    assert_eq!(done["needs"], json!([]));
    assert_eq!(done["empty_dimensions"], json!([]));
    assert_eq!(done["grid"], "confirmed");
    // It is still the track's full record.
    assert_eq!(done["energy"], 4);
    assert_eq!(done["title"], "Track 001");

    assert_eq!(track(&page, "10002")["grid"], "not_analysed");
    assert!(track(&page, "10002")["needs"]
        .as_array()
        .unwrap()
        .contains(&json!("grid")));
}

impl Fixture {
    /// Declares the `title` and `artist` kinds of Fact, as the DJ declares
    /// a kind: in the kinds file in the Sidecar's folder.
    fn declare_title_and_artist(&self) {
        let sidecar = wreckordbox_sidecar_dir(&self.location);
        std::fs::create_dir_all(&sidecar).unwrap();
        std::fs::write(
            sidecar.join("kinds.toml"),
            "[kinds.title]\nnature = \"judgement\"\nvalue = \"text\"\n\n\
             [kinds.artist]\nnature = \"judgement\"\nvalue = \"text\"\n",
        )
        .unwrap();
    }
}

/// Where a service by itself keeps the Sidecar: beside the Library.
fn wreckordbox_sidecar_dir(location: &LibraryLocation) -> std::path::PathBuf {
    location
        .master_db
        .parent()
        .unwrap()
        .join("wreckordbox-sidecar")
}

impl Hosted {
    fn record(&self, track: &str, kind: &str, value: &str, source: Value) {
        self.call(
            "facts.record",
            json!({
                "facts": [{ "track": track, "kind": kind, "value": value }],
                "source": source,
                "confidence": 0.8,
            }),
        );
    }
}

fn needs(page: &Value, id: &str) -> Vec<String> {
    track(page, id)["needs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|need| need.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_title_or_artist_an_agent_proposed_is_needed_until_the_dj_decides() {
    let fixture = Fixture::new(3);
    fixture.declare_title_and_artist();
    let hosted = fixture.start();
    let agent = json!({ "kind": "agent", "name": "tagger" });
    hosted.record("10000", "title", "A Better Title", agent.clone());
    hosted.record("10001", "artist", "Someone", agent.clone());
    // Agreeing with what the Library already says flags nothing.
    hosted.record("10002", "title", "Track 002", agent.clone());

    let page = hosted.call("intake.list", json!({}));

    assert_eq!(needs(&page, "10000")[0], "title");
    assert!(!needs(&page, "10000").contains(&"artist".to_owned()));
    assert_eq!(needs(&page, "10001")[0], "artist");
    assert!(!needs(&page, "10002").contains(&"title".to_owned()));
    let suggestion = &track(&page, "10000")["suggestions"][0];
    assert_eq!(suggestion["field"], "title");
    assert_eq!(suggestion["value"], "A Better Title");
    assert_eq!(suggestion["source"], agent);
    assert_eq!(suggestion["confidence"], 0.8);
    assert!(suggestion["fact_id"].is_i64());
    assert_eq!(track(&page, "10002")["suggestions"], json!([]));

    // The DJ's own word on the title is the decision: nothing waits.
    hosted.call(
        "facts.record",
        json!({
            "facts": [{ "track": "10000", "kind": "title", "value": "Track 000" }],
            "source": { "kind": "dj" },
        }),
    );
    let page = hosted.call("intake.list", json!({}));
    assert!(!needs(&page, "10000").contains(&"title".to_owned()));

    // A proposal made after the DJ's decision waits again.
    hosted.record("10000", "title", "Third Time Lucky", agent);
    let page = hosted.call("intake.list", json!({}));
    assert_eq!(needs(&page, "10000")[0], "title");
}

#[test]
fn asking_for_a_queue_does_not_make_a_sidecar() {
    let fixture = Fixture::new(2);
    let sidecar = wreckordbox_sidecar_dir(&fixture.location);
    let hosted = fixture.start();

    hosted.call("intake.list", json!({}));
    hosted.call("intake.counts", json!({}));

    assert!(!sidecar.exists());
}

#[test]
fn setting_a_prep_colour_moves_a_track_between_the_queues() {
    let fixture = Fixture::new(3);
    dj_labels(&fixture);
    let hosted = fixture.start();
    let events = hosted.service.events();

    // By the DJ's own name for the colour.
    let clean = hosted.call(
        "tracks.set_prep_colour",
        json!({ "id": "10000", "prep_colour": "Clean" }),
    );
    assert_eq!(clean["title"], "Track 000");
    assert_eq!(
        clean["prep_colour"],
        json!({ "id": 5, "colour": "Green", "label": "Clean" })
    );
    assert_eq!(clean["previous"], Value::Null);
    assert_eq!(clean["changed"], true);
    let told = events
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(told.event, "library_changed");
    assert_eq!(told.data["generation"], clean["generation"]);

    let intake = hosted.call("intake.list", json!({}));
    assert_eq!(ids(&intake), ["10001", "10002"]);

    hosted.call(
        "tracks.set_prep_colour",
        json!({ "id": "10001", "prep_colour": "problem" }),
    );
    assert_eq!(ids(&hosted.call("intake.list", json!({}))), ["10002"]);
    assert_eq!(ids(&hosted.call("intake.problems", json!({}))), ["10001"]);

    // By the colour itself, and back to none.
    let red = hosted.call(
        "tracks.set_prep_colour",
        json!({ "id": "10000", "prep_colour": "red" }),
    );
    assert_eq!(red["prep_colour"]["label"], "Problem");
    assert_eq!(red["previous"]["label"], "Clean");
    let none = hosted.call(
        "tracks.set_prep_colour",
        json!({ "id": "10000", "prep_colour": "none" }),
    );
    assert_eq!(none["prep_colour"], Value::Null);
    assert_eq!(
        ids(&hosted.call("intake.list", json!({}))),
        ["10000", "10002"]
    );
    let counts = hosted.call("intake.counts", json!({}));
    assert_eq!(
        (&counts["intake"], &counts["problems"]),
        (&json!(2), &json!(1))
    );
}

#[test]
fn a_prep_colour_edit_that_cannot_be_made_changes_nothing() {
    let fixture = Fixture::new(2);
    dj_labels(&fixture);
    let hosted = fixture.start();
    let generation = |hosted: &Hosted| {
        hosted.call(
            "tracks.set_prep_colour",
            json!({ "id": "10001", "prep_colour": "none" }),
        )["generation"]
            .as_u64()
            .unwrap()
    };
    let before = generation(&hosted);

    let unknown_colour = hosted
        .service
        .call(
            "tracks.set_prep_colour",
            json!({ "id": "10000", "prep_colour": "Mauve" }),
        )
        .unwrap_err();
    assert_eq!(unknown_colour.code, "prep_colour_not_found");
    assert!(
        unknown_colour.message.contains("Clean"),
        "{}",
        unknown_colour.message
    );

    let unknown_track = hosted
        .service
        .call(
            "tracks.set_prep_colour",
            json!({ "id": "77", "prep_colour": "Clean" }),
        )
        .unwrap_err();
    assert_eq!(unknown_track.code, "track_not_found");

    assert_eq!(
        ids(&hosted.call("intake.list", json!({}))),
        ["10000", "10001"]
    );
    assert_eq!(generation(&hosted), before + 1);
}

/// Every file in the Sidecar's folder, with its bytes: what a read must
/// leave exactly as it found it.
fn folder_snapshot(
    dir: &std::path::Path,
) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
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

/// Asserts the folder is byte for byte what it was, naming what changed.
fn assert_unchanged(
    dir: &std::path::Path,
    before: &std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
) {
    let after = folder_snapshot(dir);
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .collect();
    assert!(
        changed.is_empty(),
        "the read changed the Sidecar's folder: {changed:?}"
    );
}

/// A Sidecar made by an earlier service, with an agent's proposed title,
/// and a running service that did not see it at its start (it appeared
/// later): the case where only the queue's own read could open it.
fn sidecar_appearing_after_start() -> (Hosted, std::path::PathBuf) {
    let fixture = Fixture::new(3);
    fixture.declare_title_and_artist();
    let sidecar = wreckordbox_sidecar_dir(&fixture.location);
    let aside = fixture.dir.path().join("sidecar-aside");
    let location = fixture.location.clone();
    let dir = fixture.dir;
    {
        let first = Fixture {
            dir,
            location: location.clone(),
        }
        .start();
        first.record(
            "10000",
            "title",
            "A Better Title",
            json!({ "kind": "agent", "name": "tagger" }),
        );
        let Hosted { _dir: dir, service } = first;
        drop(service);
        std::fs::rename(&sidecar, &aside).unwrap();
        let hosted = Fixture { dir, location }.start();
        std::fs::rename(&aside, &sidecar).unwrap();
        (hosted, sidecar)
    }
}

#[test]
fn reading_the_queues_never_opens_migrates_or_tidies_a_sidecar() {
    let (hosted, sidecar) = sidecar_appearing_after_start();
    let before = folder_snapshot(&sidecar);
    assert!(before.keys().any(|path| path.ends_with("sidecar.db")));

    let page = hosted.call("intake.list", json!({}));
    hosted.call("intake.counts", json!({}));

    // Read through a read-only snapshot: the proposal is there...
    assert_eq!(
        track(&page, "10000")["suggestions"][0]["value"],
        "A Better Title"
    );
    assert!(page.get("suggestions_unavailable").is_none());
    // ...and not one byte of the Sidecar's folder changed, nor a file appeared.
    assert_unchanged(&sidecar, &before);
}

#[test]
fn a_sidecar_the_queue_cannot_read_safely_lists_the_queue_and_says_suggestions_are_unknown() {
    let (hosted, sidecar) = sidecar_appearing_after_start();
    // An older schema: opening it as the writer would migrate it.
    {
        let db = rusqlite::Connection::open(sidecar.join("sidecar.db")).unwrap();
        db.execute(
            "UPDATE meta SET value = '1' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();
    }
    let before = folder_snapshot(&sidecar);

    let page = hosted.call("intake.list", json!({}));

    assert_eq!(ids(&page).len(), 3);
    assert_eq!(track(&page, "10000")["suggestions"], json!([]));
    assert!(page["suggestions_unavailable"]
        .as_str()
        .is_some_and(|why| !why.is_empty()));
    assert_unchanged(&sidecar, &before);
}
