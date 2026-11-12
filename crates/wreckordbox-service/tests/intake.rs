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
        (&first["total"], &first["offset"], &first["limit"], &first["next_offset"]),
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
    assert!(refused.message.contains("\"Problem\""), "{}", refused.message);
    assert!(refused.message.contains("Clean (Green)"), "{}", refused.message);
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
    assert_eq!(counts["problems_unavailable"]["code"], "problem_colour_not_found");
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
            .execute("UPDATE djmdContent SET Analysed = NULL WHERE ID = ?1", [&fresh])
            .unwrap();
    });
    let hosted = fixture.start();

    let page = hosted.call("intake.list", json!({}));

    let untouched = track(&page, "10000");
    assert_eq!(untouched["needs"], json!(["energy", "tags", "hot_cues", "grid"]));
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
