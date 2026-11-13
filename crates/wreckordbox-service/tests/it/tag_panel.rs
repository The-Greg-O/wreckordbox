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
        let sidecar = self.sidecar_dir();
        Hosted {
            location: self.location.clone(),
            sidecar,
            _dir: self.dir,
            service,
        }
    }
}

struct Hosted {
    location: LibraryLocation,
    sidecar: std::path::PathBuf,
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
    let ids: Vec<&str> = dimensions
        .iter()
        .map(|d| d["id"].as_str().unwrap())
        .collect();
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
    record(
        json!(["techno"]),
        json!({ "kind": "agent", "name": "tagger" }),
    );
    let panel = hosted.call("intake.tag_panel", json!({ "id": "10000" }));
    assert_eq!(panel["track"]["set"]["genre"], json!(["house-deep"]));

    record(json!([]), json!({ "kind": "dj" }));
    let panel = hosted.call("intake.tag_panel", json!({ "id": "10000" }));
    assert_eq!(panel["track"]["set"]["genre"], json!([]));
}

#[test]
fn a_tag_schema_with_a_mistake_is_said_and_the_panel_falls_back_to_the_mytags() {
    let fixture = Fixture::new(1);
    fixture.write_schema(
        "version = 1\n[dimensions.mood]\nterms = [{ id = \"Bad Id\", name = \"x\" }]\n",
    );
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

// ---- wbx-65 review round: intake.set_tags ----

/// Adds a MyTag row (`attribute` 1 for a category under `root`, 0 for a
/// tag under its category), as rekordbox lists them.
fn add_my_tag(writer: &mut Writer, id: &str, seq: i64, name: &str, attribute: i64, parent: &str) {
    writer
        .library()
        .connection()
        .execute(
            "INSERT INTO djmdMyTag (ID, Seq, Name, Attribute, ParentID, UUID, rb_data_status,
                rb_local_data_status, rb_local_deleted, rb_local_synced, usn, rb_local_usn,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'fixture-' || ?1, 0, 0, 0, 0, NULL, 1, '2026-01-01', '2026-01-01')",
            (id, seq, name, attribute, parent),
        )
        .unwrap();
}

/// A sub-genre category written as the DJ writes it flat: House, its two
/// children, Techno.
fn sub_genre_category(fixture: &Fixture) {
    fixture.edit(|writer| {
        add_my_tag(writer, "800000", 2, "Sub-genre", 1, "root");
        add_my_tag(writer, "800001", 1, "House", 0, "800000");
        add_my_tag(writer, "800002", 2, "> Deep", 0, "800000");
        add_my_tag(writer, "800003", 3, "> Acid", 0, "800000");
        add_my_tag(writer, "800004", 4, "Techno", 0, "800000");
    });
}

fn deck_tree_schema(tag_parents: bool) -> String {
    format!(
        r#"
version = 1

[genre]
terms = [
  {{ id = "house", name = "House" }},
  {{ id = "house-deep", name = "Deep", parent = "house" }},
  {{ id = "house-acid", name = "Acid", parent = "house" }},
  {{ id = "techno", name = "Techno" }},
]

[dimensions.situation]
terms = [
  {{ id = "peak", name = "Peak" }},
  {{ id = "warm-up", name = "Warm-up" }},
]

[dimensions.vocals]
terms = [
  {{ id = "male", name = "Male" }},
  {{ id = "female", name = "Female" }},
]

[dimensions.mood]
terms = [
  {{ id = "dark", name = "Dark" }},
  {{ id = "bright", name = "Bright" }},
]

[deck]
categories = [
  {{ dimension = "genre", label = "Sub-genre" }},
  {{ dimension = "situation", label = "Situation" }},
]

[deck.genre]
child_prefix = "> "
tag_parents = {tag_parents}
"#
    )
}

impl Hosted {
    fn panel(&self, track: &str) -> Value {
        self.call("intake.tag_panel", json!({ "id": track }))
    }

    /// The click of the panel: the dimension set to `terms`, from the
    /// version the panel shows now.
    fn set(&self, track: &str, dimension: &str, terms: &[&str]) -> Value {
        let version = self.panel(track)["track"]["versions"][dimension].clone();
        self.call(
            "intake.set_tags",
            json!({ "id": track, "dimension": dimension, "terms": terms, "version": version }),
        )
    }

    fn try_set(&self, args: Value) -> wreckordbox_service::ServiceError {
        self.service.call("intake.set_tags", args).unwrap_err()
    }

    /// The track's MyTag ids, as the Library holds them.
    fn my_tags(&self, track: &str) -> Vec<String> {
        let db = rbl_db::Library::open(self.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
        let mut statement = db
            .connection()
            .prepare("SELECT MyTagID FROM djmdSongMyTag WHERE ContentID = ?1 AND rb_local_deleted = 0 ORDER BY MyTagID")
            .unwrap();
        statement
            .query_map([track], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    /// The DJ's live Fact of `kind` on `track`: its value, or null.
    fn djs(&self, track: &str, kind: &str) -> Value {
        let facts = self.call("facts.of_track", json!({ "id": track }));
        facts["facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|fact| fact["kind"] == kind && fact["source"]["kind"] == "dj")
            .filter(|fact| fact["superseded"] != true)
            .max_by_key(|fact| fact["id"].as_i64().unwrap())
            .map_or(Value::Null, |fact| fact["value"].clone())
    }
}

fn start_with(fixture: Fixture) -> Hosted {
    fixture.start()
}

#[test]
fn a_deck_tree_with_tag_parents_writes_the_parents_mytags_and_narrows_on_the_parent() {
    let fixture = Fixture::new(2);
    sub_genre_category(&fixture);
    fixture.write_schema(&deck_tree_schema(true));
    let hosted = start_with(fixture);
    let panel = hosted.panel("10000");
    assert_eq!(panel["dimensions"][0]["id"], "genre");
    assert_eq!(panel["dimensions"][0]["tag_parents"], true);

    // A child: its MyTag and its parent's, one edit; the Fact names the child.
    let set = hosted.set("10000", "genre", &["house-deep"]);
    assert_eq!(set["changed"], true);
    assert_eq!(hosted.my_tags("10000"), ["800001", "800002"]);
    assert_eq!(set["panel"]["track"]["set"]["genre"], json!(["house-deep"]));
    assert_eq!(hosted.djs("10000", "genre"), json!(["house-deep"]));

    // A sibling keeps the shared parent; clearing one child keeps it for
    // the other; clearing the last clears the parent nobody needs.
    hosted.set("10000", "genre", &["house-deep", "house-acid"]);
    assert_eq!(hosted.my_tags("10000"), ["800001", "800002", "800003"]);
    hosted.set("10000", "genre", &["house-acid"]);
    assert_eq!(hosted.my_tags("10000"), ["800001", "800003"]);
    hosted.set("10000", "genre", &[]);
    assert_eq!(hosted.my_tags("10000"), Vec::<String>::new());

    // Narrowing to the parent: the parent alone.
    hosted.set("10000", "genre", &["house-deep"]);
    hosted.set("10000", "genre", &["house"]);
    assert_eq!(hosted.my_tags("10000"), ["800001"]);
    assert_eq!(hosted.djs("10000", "genre"), json!(["house"]));
    // A tag and its own ancestor is the tag alone.
    hosted.set("10000", "genre", &["house", "house-acid"]);
    assert_eq!(
        hosted.panel("10000")["track"]["set"]["genre"],
        json!(["house-acid"])
    );
}

#[test]
fn a_deck_tree_without_tag_parents_writes_only_the_childs_mytag() {
    let fixture = Fixture::new(1);
    sub_genre_category(&fixture);
    fixture.write_schema(&deck_tree_schema(false));
    let hosted = start_with(fixture);
    assert_eq!(hosted.panel("10000")["dimensions"][0]["tag_parents"], false);

    hosted.set("10000", "genre", &["house-deep"]);
    assert_eq!(hosted.my_tags("10000"), ["800002"]);
    assert_eq!(
        hosted.panel("10000")["track"]["set"]["genre"],
        json!(["house-deep"])
    );
    assert_eq!(hosted.djs("10000", "genre"), json!(["house-deep"]));

    hosted.set("10000", "genre", &["house"]);
    assert_eq!(hosted.my_tags("10000"), ["800001"]);
}

#[test]
fn a_vocabulary_dimension_is_set_as_the_djs_fact_and_a_scalar_replaces_and_clears() {
    let fixture = Fixture::new(1);
    sub_genre_category(&fixture);
    fixture.write_schema(&deck_tree_schema(true));
    let hosted = start_with(fixture);
    let panel = hosted.panel("10000");
    let vocals = panel["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "vocals")
        .unwrap();
    assert_eq!(vocals["edit"], "fact");
    assert_eq!(vocals["single"], true);

    hosted.set("10000", "mood", &["dark", "bright"]);
    assert_eq!(hosted.djs("10000", "mood"), json!(["dark", "bright"]));

    hosted.set("10000", "vocals", &["male"]);
    assert_eq!(hosted.djs("10000", "vocals"), json!("male"));
    hosted.set("10000", "vocals", &["female"]);
    assert_eq!(hosted.djs("10000", "vocals"), json!("female"));
    // Two for a dimension that takes one: refused.
    let version = hosted.panel("10000")["track"]["versions"]["vocals"].clone();
    let two = hosted.try_set(json!({ "id": "10000", "dimension": "vocals", "terms": ["male", "female"], "version": version }));
    assert_eq!(two.code, "tag_not_in_dimension");
    // Clearing withdraws the DJ's opinion: no live DJ Fact, nothing set.
    hosted.set("10000", "vocals", &[]);
    assert_eq!(hosted.djs("10000", "vocals"), Value::Null);
    assert_eq!(hosted.panel("10000")["track"]["set"]["vocals"], json!([]));
}

#[test]
fn a_click_from_a_stale_panel_or_an_unknown_tag_is_refused_and_changes_nothing() {
    let fixture = Fixture::new(1);
    sub_genre_category(&fixture);
    fixture.write_schema(&deck_tree_schema(true));
    let sidecar = fixture.sidecar_dir();
    let hosted = start_with(fixture);
    let old = hosted.panel("10000")["track"]["versions"]["mood"].clone();

    // The DJ moves mood onto the deck while the panel is open.
    std::fs::write(
        sidecar.join("tag-schema.toml"),
        deck_tree_schema(true).replace(
            r#"{ dimension = "situation", label = "Situation" },"#,
            r#"{ dimension = "situation", label = "Situation" }, { dimension = "mood", label = "Mood" },"#,
        ),
    )
    .unwrap();
    let stale = hosted
        .try_set(json!({ "id": "10000", "dimension": "mood", "terms": ["dark"], "version": old }));
    assert_eq!(stale.code, "tag_panel_stale");
    assert_eq!(hosted.djs("10000", "mood"), Value::Null);
    let panel = hosted.panel("10000");
    let mood = panel["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "mood")
        .unwrap();
    assert_eq!(mood["edit"], "my_tag");

    // A tag the dimension does not have.
    let version = hosted.panel("10000")["track"]["versions"]["genre"].clone();
    let unknown = hosted.try_set(
        json!({ "id": "10000", "dimension": "genre", "terms": ["polka"], "version": version }),
    );
    assert_eq!(unknown.code, "tag_not_in_dimension");
    // A deck tag with no MyTag in the Library yet (Mood is not projected).
    let version = hosted.panel("10000")["track"]["versions"]["mood"].clone();
    let not_on_deck = hosted.try_set(
        json!({ "id": "10000", "dimension": "mood", "terms": ["dark"], "version": version }),
    );
    assert_eq!(not_on_deck.code, "tag_not_settable");
    assert_eq!(hosted.my_tags("10000"), Vec::<String>::new());
}

#[test]
fn values_no_tag_stands_for_are_shown_and_kept_unless_dropped() {
    let fixture = Fixture::new(1);
    sub_genre_category(&fixture);
    fixture.edit(|writer| {
        add_my_tag(writer, "800005", 5, "Breaks", 0, "800000");
        writer
            .set_my_tags(&fixture::track_id(0), &["800005".to_owned()])
            .unwrap();
    });
    fixture.write_schema(&deck_tree_schema(true));
    let hosted = start_with(fixture);
    // A removed term in the DJ's Fact (recorded before the schema dropped it).
    hosted.call(
        "facts.record",
        json!({ "facts": [{ "track": "10000", "kind": "mood", "value": ["dark"] }], "source": { "kind": "dj" } }),
    );
    let schema = deck_tree_schema(true).replace(r#"  { id = "dark", name = "Dark" },"#, "");
    std::fs::write(hosted.sidecar.join("tag-schema.toml"), schema).unwrap();

    let panel = hosted.panel("10000");
    assert_eq!(
        panel["track"]["unmapped"]["genre"],
        json!([{ "id": "800005", "name": "Breaks" }])
    );
    assert_eq!(
        panel["track"]["unmapped"]["mood"],
        json!([{ "id": "dark", "name": "dark" }])
    );

    // A replacement keeps them.
    hosted.set("10000", "genre", &["techno"]);
    assert_eq!(hosted.my_tags("10000"), ["800004", "800005"]);
    hosted.set("10000", "mood", &["bright"]);
    assert_eq!(hosted.djs("10000", "mood"), json!(["bright", "dark"]));

    // Taken off only when the DJ names them.
    let version = hosted.panel("10000")["track"]["versions"]["mood"].clone();
    hosted.call(
        "intake.set_tags",
        json!({ "id": "10000", "dimension": "mood", "terms": ["bright"], "version": version, "drop_unmapped": ["dark"] }),
    );
    assert_eq!(hosted.djs("10000", "mood"), json!(["bright"]));
    let version = hosted.panel("10000")["track"]["versions"]["genre"].clone();
    hosted.call(
        "intake.set_tags",
        json!({ "id": "10000", "dimension": "genre", "terms": ["techno"], "version": version, "drop_unmapped": ["800005"] }),
    );
    assert_eq!(hosted.my_tags("10000"), ["800004"]);
}

#[test]
fn a_deck_edit_whose_fact_cannot_be_derived_says_so() {
    // Breaks is a MyTag of the deck category that no term stands for, so
    // the native capture cannot map the category and skips its Fact.
    let fixture = Fixture::new(1);
    sub_genre_category(&fixture);
    fixture.edit(|writer| {
        add_my_tag(writer, "800005", 5, "Breaks", 0, "800000");
        writer
            .set_my_tags(&fixture::track_id(0), &["800005".to_owned()])
            .unwrap();
    });
    fixture.write_schema(&deck_tree_schema(true));
    let hosted = start_with(fixture);

    let set = hosted.set("10000", "genre", &["techno"]);

    assert_eq!(hosted.my_tags("10000"), ["800004", "800005"]);
    let reasons = set["facts_not_recorded"].as_array().unwrap();
    assert!(!reasons.is_empty(), "{set}");
}

#[test]
fn the_queue_asks_for_tags_in_the_dimensions_the_panel_offers_as_deck_facing() {
    let fixture = Fixture::new(1);
    sub_genre_category(&fixture);
    // Situation is not on this schema's deck; Mood is, with no MyTags yet.
    fixture.write_schema(&deck_tree_schema(true).replace(
        r#"{ dimension = "situation", label = "Situation" },"#,
        r#"{ dimension = "mood", label = "Mood" },"#,
    ));
    fixture.edit(|writer| {
        writer
            .set_my_tags(
                &fixture::track_id(0),
                &[fixture::MY_TAG_PEAK.to_owned(), "800004".to_owned()],
            )
            .unwrap();
    });
    let hosted = start_with(fixture);

    let queue = hosted.call("intake.list", json!({}));
    let empty: Vec<&str> = queue["tracks"][0]["empty_dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();

    // Genre is tagged (Techno); Mood is empty; Situation is not asked for.
    assert_eq!(empty, ["Mood"]);
    let panel = hosted.panel("10000");
    let deck: Vec<&str> = panel["dimensions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["deck"] == true)
        .map(|d| d["label"].as_str().unwrap())
        .collect();
    assert_eq!(deck, ["Sub-genre", "Mood"]);
}

#[test]
fn reading_the_djs_facts_from_a_cold_sidecar_changes_no_byte_of_it() {
    let fixture = Fixture::new(1);
    fixture.write_schema(FIVE);
    let location = fixture.location.clone();
    let sidecar = fixture.sidecar_dir();
    let dir = fixture.dir;
    let config = Config {
        runtime_dir: dir.path().join("run"),
        idle_exit: None,
        recovery_dir: dir.path().join("recovery"),
    };
    wreckordbox_service::protection::store(&config.recovery_dir, false).unwrap();
    {
        let first = Service::start(location.clone(), &config).unwrap();
        first
            .call(
                "facts.record",
                json!({ "facts": [{ "track": "10000", "kind": "genre", "value": ["techno"] }], "source": { "kind": "dj" } }),
            )
            .unwrap();
        first.stop();
    }
    let bytes = |dir: &std::path::Path| -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| (path.display().to_string(), std::fs::read(&path).unwrap()))
            .collect();
        files.sort();
        files
    };
    let second = Service::start(location, &config).unwrap();
    // The service's own start-up (Ledger recovery) may touch the Sidecar;
    // the read must not.
    let before = bytes(&sidecar);

    let panel = second
        .call("intake.tag_panel", json!({ "id": "10000" }))
        .unwrap();

    assert_eq!(panel["track"]["set"]["genre"], json!(["techno"]));
    assert_eq!(panel["track"]["facts_unavailable"], Value::Null);
    assert_eq!(bytes(&sidecar), before);
}
