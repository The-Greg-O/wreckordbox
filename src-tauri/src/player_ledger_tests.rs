//! Player messages through the real LINK adapter, observed by the hosted CLI.
#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::file_ledger_tests::Fixture;
use serde_json::{json, Value};

fn source(f: &Fixture) -> StateSource {
    StateSource(
        Arc::downgrade(&f.state),
        Arc::new(|_, _| {}),
        rbl_prolink::DeviceSettings::default(),
        rbl_link::KeyOrder::Musical,
        Arc::default(),
    )
}

#[test]
fn link_rating_is_a_player_entry_with_values_and_no_dj_fact() {
    let f = Fixture::new(3);
    let source = source(&f);
    let id = rbl_db::fixture::track_id(1);
    let before = f
        .state
        .read_db(|db| {
            Ok(db.connection().query_row(
                "SELECT Rating FROM djmdContent WHERE ID=?1",
                [&id],
                |r| r.get::<_, u8>(0),
            )?)
        })
        .unwrap();
    let stars = (before + 1) % 6;
    assert!(source.edit(&rbl_link::Edit::Rating {
        track: id.parse().unwrap(),
        stars
    }));
    let runs = f.cli(&["runs", "list"]);
    assert_eq!(
        runs["total"], 1,
        "the accepted player write needs its own Run"
    );
    assert_eq!(
        runs["runs"][0]["source"],
        json!({"kind":"tool","name":"link"})
    );
    let run = runs["runs"][0]["id"].to_string();
    let shown = f.cli(&["runs", "show", &run]);
    let entries = shown["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"], "track");
    assert_eq!(
        entries[0]["payload"]["edits"],
        json!([{"track":id,"column":"Rating","before":before,"after":stars}])
    );
    assert_eq!(entries[0]["source"], json!({"kind":"tool","name":"link"}));
    let facts = f.cli(&["facts", "show", &id, "--all"]);
    assert_eq!(
        facts["facts"],
        json!([]),
        "a player's rating is never a DJ Fact"
    );
}

#[test]
fn link_cues_tags_and_history_are_named_and_protection_refuses_recording() {
    let f = Fixture::new(3);
    let source = source(&f);
    let id = rbl_db::fixture::track_id(1);
    let track = id.parse().unwrap();
    f.state.core().set_protection(true).unwrap();
    assert!(!source.edit(&rbl_link::Edit::Rating { track, stars: 5 }));
    assert_eq!(f.cli(&["runs", "list"])["total"], 0);
    f.state.core().set_protection(false).unwrap();
    assert!(source.edit(&rbl_link::Edit::Tag { track, add: true }));
    assert!(source.edit(&rbl_link::Edit::SaveCue {
        track,
        cue: rbl_link::PlayerCue {
            in_ms: 1234,
            ..Default::default()
        }
    }));
    let history = source.new_link_history().unwrap();
    assert!(source.add_to_history(history, track));
    assert!(source.remove_from_history(history, track));
    assert!(source.edit(&rbl_link::Edit::Tag { track, add: false }));
    assert!(source.edit(&rbl_link::Edit::Tag { track, add: true }));
    assert!(source.edit(&rbl_link::Edit::ClearTags));
    assert!(source.edit(&rbl_link::Edit::DeleteCue {
        track,
        cue: rbl_link::PlayerCue {
            in_ms: 1234,
            ..Default::default()
        }
    }));
    let runs = f.cli(&["runs", "list"]);
    let mut operations = Vec::new();
    for run in runs["runs"].as_array().unwrap() {
        assert_eq!(run["source"], json!({"kind":"tool","name":"link"}));
        let shown = f.cli(&["runs", "show", &run["id"].to_string()]);
        for entry in shown["entries"].as_array().unwrap() {
            assert_eq!(entry["reversible"], false);
            assert!(entry["payload"]["not_reversible_reason"].as_str().is_some());
            operations.push(entry["payload"]["records"][0]["operation"].clone());
        }
    }
    assert_eq!(
        operations.len(),
        9,
        "every accepted LINK row write must be named: {operations:?}"
    );
    assert!(operations.contains(&Value::from("tag_add")));
    assert!(operations.contains(&Value::from("cue_save")));
    assert!(operations.contains(&Value::from("history_create")));
    assert!(operations.contains(&Value::from("history_add")));
    for name in ["history_remove", "tag_remove", "tag_clear", "cue_delete"] {
        assert!(
            operations.contains(&Value::from(name)),
            "{name}: {operations:?}"
        );
    }
}

#[test]
fn round_trip_cues_and_history_share_one_player_run_and_leave_the_stick_untouched() {
    let f = Fixture::new(3);
    let id = rbl_db::fixture::track_id(1);
    let db_id = f
        .state
        .read_db(|db| rbl_db::export_info::db_id(db.connection()))
        .unwrap();
    let usb = f.dir.path().join("stick");
    let one = usb.join("PIONEER/rekordbox/exportLibrary.db");
    std::fs::create_dir_all(one.parent().unwrap()).unwrap();
    let relative = "PIONEER/USBANLZ/player/ANLZ0000.DAT";
    let dat = usb.join(relative);
    std::fs::create_dir_all(dat.parent().unwrap()).unwrap();
    let mut analysis = rbl_anlz::AnlzBuilder::new();
    analysis
        .beat_grid(&[rbl_anlz::Beat {
            beat_number: 1,
            tempo_x100: 13000,
            time_ms: 200,
        }])
        .cue_lists(true);
    std::fs::write(&dat, analysis.finish()).unwrap();
    let mut builder = rbl_onelibrary::build::Builder::create(&one).unwrap();
    builder
        .add_track(&rbl_onelibrary::build::Track {
            content_id: 1,
            master_db_id: i64::try_from(db_id).unwrap(),
            master_content_id: id.parse().unwrap(),
            title: "Fixture".into(),
            path: "/Contents/fixture.mp3".into(),
            analysis_path: format!("/{relative}"),
            ..Default::default()
        })
        .unwrap();
    let mut destinations = vec![f.dat.clone()];
    for (export_id, index) in [(2, 0), (3, 2)] {
        let path = format!("/PIONEER/USBANLZ/fixture-{index}/ANLZ0000.DAT");
        rbl_db::fixture::set_analysis_path(&f.location, index, &path).unwrap();
        let destination = rbl_anlz::resolve(&f.location.share_root, &path);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::copy(&f.dat, &destination).unwrap();
        destinations.push(destination);
        builder
            .add_track(&rbl_onelibrary::build::Track {
                content_id: export_id,
                master_db_id: i64::try_from(db_id).unwrap(),
                master_content_id: rbl_db::fixture::track_id(index).parse().unwrap(),
                title: format!("Fixture {index}"),
                path: format!("/Contents/fixture-{index}.mp3"),
                analysis_path: format!("/{relative}"),
                ..Default::default()
            })
            .unwrap();
    }
    builder.add_history(1, "Player set", 0, 1, false).unwrap();
    builder.add_history_track(1, 1, 1).unwrap();
    builder.finish("Fixture", "2026-11-13", 0).unwrap();
    let before_stick = (std::fs::read(&one).unwrap(), std::fs::read(&dat).unwrap());
    let editor = crate::grid::GridEditor::at(f.state.backup_dir());
    crate::usb_import::import(&f.state, &editor, &usb, true, true, false, false).unwrap();
    for destination in destinations {
        assert_eq!(
            rbl_anlz::Anlz::read(&destination)
                .unwrap()
                .beat_grid()
                .unwrap()[0]
                .time_ms,
            200
        );
    }
    assert_eq!(
        (std::fs::read(&one).unwrap(), std::fs::read(&dat).unwrap()),
        before_stick
    );
    let runs = f.cli(&["runs", "list"]);
    assert_eq!(
        runs["total"], 1,
        "one operation, not a Run per track or history"
    );
    assert_eq!(
        runs["runs"][0]["source"],
        json!({"kind":"tool","name":"round-trip"})
    );
    let shown = f.cli(&["runs", "show", &runs["runs"][0]["id"].to_string()]);
    assert_eq!(shown["total"], 4, "{shown}");
    let operations: Vec<_> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["payload"]["records"][0]["operation"].clone())
        .collect();
    assert_eq!(
        operations,
        vec![
            json!("cue_grid"),
            json!("cue_grid"),
            json!("cue_grid"),
            json!("history_import")
        ]
    );
    let mut recorded: Vec<_> = shown["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| {
            entry["payload"]["records"][0]["track"]
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    recorded.sort();
    assert_eq!(
        recorded,
        (0..3).map(rbl_db::fixture::track_id).collect::<Vec<_>>()
    );
    crate::usb_import::import(&f.state, &editor, &usb, true, true, false, false).unwrap();
    assert_eq!(
        f.cli(&["runs", "list"])["total"],
        1,
        "unchanged Round-trip creates no Run"
    );
}

fn project_rating(f: &Fixture, id: &str, stars: u8) -> Value {
    f.cli(&[
        "facts",
        "record",
        id,
        "energy",
        &stars.to_string(),
        "--source",
        "agent:fixture",
        "--confidence",
        "1",
    ]);
    std::fs::write(
        wreckordbox_sidecar::default_dir(&f.location.master_db).join("projection-policy.toml"),
        "[energy]\napply = 0.8\n",
    )
    .unwrap();
    f.cli(&["project"])
}

#[test]
fn a_same_value_link_rating_ends_projection_ownership() {
    let f = Fixture::new(3);
    let id = rbl_db::fixture::track_id(1);
    f.state.write(|w| w.set_rating(&id, 0)).unwrap();
    f.state
        .read_db(|db| {
            crate::commands::refresh_after_edit(
                &f.state,
                db,
                crate::commands::Touched::Metadata(vec![id.clone()]),
            )
        })
        .unwrap();
    assert_eq!(
        project_rating(&f, &id, 3)["changes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(source(&f).edit(&rbl_link::Edit::Rating {
        track: id.parse().unwrap(),
        stars: 3
    }));
    let applied = project_rating(&f, &id, 4);
    assert_eq!(applied["changes"], json!([]), "{applied}");
    assert_eq!(
        f.state.library().unwrap().rating[f.state.library().unwrap().row_of(&id).unwrap() as usize],
        3
    );
    assert!(f.cli(&["facts", "show", &id, "--all"])["facts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|fact| fact["source"]["kind"] != "dj"));
}

#[test]
fn a_link_clear_committed_before_refresh_failure_bars_projection() {
    let f = Fixture::new(3);
    let id = rbl_db::fixture::track_id(1);
    f.state.write(|w| w.set_rating(&id, 2)).unwrap();
    // Model an index that has not yet observed this live row. The real
    // refresh fails after the writer commits because row_of cannot find it.
    let db = rbl_db::Library::open(f.location.clone(), rbl_db::OpenMode::ReadWrite).unwrap();
    db.connection()
        .execute(
            "UPDATE djmdContent SET rb_local_deleted=1 WHERE ID=?1",
            [&id],
        )
        .unwrap();
    let (library, _) = rbl_index::load(&db).unwrap();
    f.state.set_library(
        library,
        false,
        db.schema().db_version,
        0,
        f.location.clone(),
    );
    db.connection()
        .execute(
            "UPDATE djmdContent SET rb_local_deleted=0 WHERE ID=?1",
            [&id],
        )
        .unwrap();
    assert!(!source(&f).edit(&rbl_link::Edit::Rating {
        track: id.parse().unwrap(),
        stars: 0
    }));
    assert_eq!(
        db.connection()
            .query_row("SELECT Rating FROM djmdContent WHERE ID=?1", [&id], |r| r
                .get::<_, u8>(0))
            .unwrap(),
        0
    );
    let (library, _) = rbl_index::load(&db).unwrap();
    f.state.set_library(
        library,
        false,
        db.schema().db_version,
        0,
        f.location.clone(),
    );
    let applied = project_rating(&f, &id, 4);
    assert_eq!(applied["changes"], json!([]), "{applied}");
}
