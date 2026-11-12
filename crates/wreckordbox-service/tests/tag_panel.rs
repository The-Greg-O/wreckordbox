//! The Intake tag panel's description (wbx-65), asked of a service in the
//! test's own process through its command interface. The Library is a
//! throwaway fixture; a Tag schema, when a test wants one, is written into
//! the Sidecar's folder as the DJ writes it.
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

    fn edit(&self, change: impl FnOnce(&mut Writer)) {
        let mut writer =
            Writer::open(self.location.clone(), self.dir.path().join("backups")).unwrap();
        writer.disable_automatic_backups();
        change(&mut writer);
    }

    fn sidecar_dir(&self) -> std::path::PathBuf {
        self.location
            .master_db
            .parent()
            .unwrap()
            .join("wreckordbox-sidecar")
    }

    fn write_schema(&self, text: &str) {
        let dir = self.sidecar_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tag-schema.toml"), text).unwrap();
    }

    fn start(self) -> Hosted {
        let config = Config {
            runtime_dir: self.dir.path().join("run"),
            idle_exit: None,
            recovery_dir: self.dir.path().join("recovery"),
        };
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

/// `label: tag, tag...` for each dimension, deck-facing ones marked `*`:
/// what the DJ would read off the panel.
fn read_off(panel: &Value) -> Vec<String> {
    panel["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|dimension| {
            let tags: Vec<String> = dimension["tags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tag| {
                    format!(
                        "{}{}",
                        "  ".repeat(tag["depth"].as_u64().unwrap() as usize),
                        tag["name"].as_str().unwrap()
                    )
                })
                .collect();
            format!(
                "{}{}: {}",
                if dimension["deck"] == true { "*" } else { "" },
                dimension["label"].as_str().unwrap(),
                tags.join(", ")
            )
        })
        .collect()
}

#[test]
fn with_no_tag_schema_the_panel_is_the_librarys_mytag_categories() {
    let fixture = Fixture::new(2);
    fixture.edit(|writer| {
        writer
            .set_my_tags(&fixture::track_id(1), &[fixture::MY_TAG_PEAK.to_owned()])
            .unwrap();
    });
    let hosted = fixture.start();

    let panel = hosted.call("intake.tag_panel", json!({ "id": "10001" }));

    assert_eq!(panel["source"], "my_tags");
    assert_eq!(panel["schema_problem"], Value::Null);
    assert_eq!(read_off(&panel), ["*Situation: Peak, Warm-up"]);
    let situation = &panel["dimensions"][0];
    assert_eq!(situation["edit"], "my_tag");
    assert_eq!(situation["tags"][0]["my_tag"], fixture::MY_TAG_PEAK);
    assert_eq!(situation["tags"][1]["my_tag"], fixture::MY_TAG_WARM_UP);
    // What the track carries, by the dimension's and the tags' identifiers.
    let id = situation["id"].as_str().unwrap();
    assert_eq!(panel["track"]["id"], "10001");
    assert_eq!(
        panel["track"]["set"][id],
        json!([situation["tags"][0]["id"]])
    );

    let bare = hosted.call("intake.tag_panel", json!({ "id": "10000" }));
    assert_eq!(bare["track"]["set"][id], json!([]));
    // Asked without a track, it is the description alone.
    assert_eq!(
        hosted.call("intake.tag_panel", json!({}))["track"],
        Value::Null
    );
}

/// Five dimensions, none named as rekordbox's MyTag categories are except
/// the one the fixture has: a tree, two on the deck, two vocabulary only.
const FIVE: &str = r#"
version = 1

[genre]
terms = [
  { id = "house", name = "House" },
  { id = "house-deep", name = "Deep", parent = "house" },
  { id = "techno", name = "Techno" },
]

[dimensions.situation]
terms = [
  { id = "peak", name = "Peak" },
  { id = "warm-up", name = "Warm-up" },
]

[dimensions.feel]
terms = [
  { id = "dark", name = "Dark" },
  { id = "bright", name = "Bright" },
]

[dimensions.texture]
terms = [{ id = "gritty", name = "Gritty" }]

[dimensions.set_role]
terms = [{ id = "closer", name = "Closer" }]

[deck]
categories = [
  { dimension = "feel", label = "Mood" },
  { dimension = "situation", label = "Situation", order = ["warm-up"] },
]
"#;

#[test]
fn a_tag_schema_draws_the_panel_with_the_deck_facing_dimensions_first() {
    let fixture = Fixture::new(2);
    fixture.write_schema(FIVE);
    fixture.edit(|writer| {
        writer
            .set_my_tags(&fixture::track_id(0), &[fixture::MY_TAG_PEAK.to_owned()])
            .unwrap();
    });
    let hosted = fixture.start();

    let panel = hosted.call("intake.tag_panel", json!({ "id": "10000" }));

    assert_eq!(panel["source"], "tag_schema");
    assert_eq!(
        read_off(&panel),
        [
            "*Mood: Dark, Bright",
            "*Situation: Warm-up, Peak",
            "Genre: House,   Deep, Techno",
            "Set role: Closer",
            "Texture: Gritty",
        ]
    );
    let dimensions = panel["dimensions"].as_array().unwrap();
    let ids: Vec<&str> = dimensions.iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["feel", "situation", "genre", "set_role", "texture"]);
    // The deck's MyTags are edited as MyTags, matched by category and text;
    // a deck category the Library has no MyTag category for cannot be set
    // until the schema is projected.
    assert_eq!(dimensions[0]["edit"], "my_tag");
    assert_eq!(dimensions[0]["tags"][0]["my_tag"], Value::Null);
    assert_eq!(dimensions[1]["tags"][1]["id"], "peak");
    assert_eq!(dimensions[1]["tags"][1]["my_tag"], fixture::MY_TAG_PEAK);
    // The rest are the DJ's Facts.
    assert_eq!(dimensions[2]["edit"], "fact");
    assert_eq!(dimensions[2]["tree"], true);
    assert_eq!(dimensions[2]["tags"][1]["parent"], "house");
    assert_eq!(panel["track"]["set"]["situation"], json!(["peak"]));
    assert_eq!(panel["track"]["set"]["genre"], json!([]));
}

#[test]
fn a_vocabulary_dimension_is_set_by_the_djs_own_fact_not_an_agents() {
    let fixture = Fixture::new(1);
    fixture.write_schema(FIVE);
    let hosted = fixture.start();
    let record = |value: Value, source: Value| {
        hosted.call(
            "facts.record",
            json!({
                "facts": [{ "track": "10000", "kind": "genre", "value": value }],
                "source": source,
                "confidence": 0.9,
            }),
        );
    };

    record(json!(["house-deep"]), json!({ "kind": "dj" }));
    record(json!(["techno"]), json!({ "kind": "agent", "name": "tagger" }));
    let panel = hosted.call("intake.tag_panel", json!({ "id": "10000" }));
    assert_eq!(panel["track"]["set"]["genre"], json!(["house-deep"]));

    record(json!([]), json!({ "kind": "dj" }));
    let panel = hosted.call("intake.tag_panel", json!({ "id": "10000" }));
    assert_eq!(panel["track"]["set"]["genre"], json!([]));
}

#[test]
fn a_tag_schema_with_a_mistake_is_said_and_the_panel_falls_back_to_the_mytags() {
    let fixture = Fixture::new(1);
    fixture.write_schema("version = 1\n[dimensions.mood]\nterms = [{ id = \"Bad Id\", name = \"x\" }]\n");
    let hosted = fixture.start();

    let panel = hosted.call("intake.tag_panel", json!({ "id": "10000" }));

    assert_eq!(panel["source"], "my_tags");
    assert!(panel["schema_problem"]
        .as_str()
        .unwrap()
        .contains("tag-schema.toml"));
    assert_eq!(read_off(&panel), ["*Situation: Peak, Warm-up"]);
}

#[test]
fn reading_the_panel_writes_nothing_and_does_not_make_a_sidecar() {
    let fixture = Fixture::new(1);
    let sidecar = fixture.sidecar_dir();
    let hosted = fixture.start();

    hosted.call("intake.tag_panel", json!({ "id": "10000" }));

    assert!(!sidecar.exists());
}

#[test]
fn an_unknown_track_is_refused() {
    let hosted = Fixture::new(1).start();

    let error = hosted
        .service
        .call("intake.tag_panel", json!({ "id": "99999" }))
        .unwrap_err();

    assert_eq!(error.code, "track_not_found");
}
