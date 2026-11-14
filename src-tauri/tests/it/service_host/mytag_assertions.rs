//! The window's MyTag tick is the DJ's hand set (wbx-29 review): across a
//! selection, the tracks already in the ticked state are the DJ's
//! assertion of it, while the Undo step holds only the tracks it changed;
//! a MyTag projection never takes back what the DJ confirmed.

use super::*;

use rbl_db::fixture::{MY_TAG_PEAK, MY_TAG_WARM_UP};

const SCHEMA: &str = r#"version = 1

[dimensions.situation]
terms = [
  { id = "peak", name = "Peak" },
  { id = "warm-up", name = "Warm-up" },
]

[deck]
categories = [
  { dimension = "situation", label = "Situation" },
]
"#;

#[test]
fn the_djs_tick_of_a_selection_asserts_the_tracks_already_ticked_and_undo_holds_the_changes() {
    let window = window();
    window.host().unwrap();
    let (a, b) = (track_id(1), track_id(2));
    let sidecar = window.location.master_db.parent().unwrap().join("wreckordbox-sidecar");
    std::fs::create_dir_all(&sidecar).unwrap();
    std::fs::write(sidecar.join("projection-policy.toml"), "[mytags]\napply = 0.5\n").unwrap();
    std::fs::write(sidecar.join("tag-schema.toml"), SCHEMA).unwrap();
    let fact = |track: &str, term: &str| {
        let run = window.cli(&["--json", "facts", "record", track, "situation", term, "--source", "agent:tagger", "--confidence", "0.9"]);
        assert!(run.ok, "{}", run.stderr);
    };
    let tags = |track: &str| -> Vec<String> {
        let mut tags: Vec<String> = window.cli(&["--json", "mytags", "show", track]).json()["my_tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tag| tag["name"].as_str().unwrap().to_owned())
            .collect();
        tags.sort();
        tags
    };
    fact(&a, "peak");
    let projected = window.cli(&["--json", "project", "--mytags"]).json();
    assert_eq!(projected["changes"].as_array().unwrap().len(), 1, "{projected}");
    assert_eq!(tags(&a), ["Peak"]);

    // The DJ ticks Peak on a (it is on) and b (it is not), in the window.
    let handle = || window.app.handle().clone();
    run(rbxport_lib::details::put_my_tag(handle(), window.state(), vec![a.clone(), b.clone()], MY_TAG_PEAK.to_owned(), true, None)).unwrap();
    assert_eq!(tags(&b), ["Peak"]);
    // Undo takes back b's change only; a stays as the DJ confirmed it.
    run(commands::undo_edit(handle(), window.state())).unwrap();
    assert_eq!(tags(&b), Vec::<String>::new());
    assert_eq!(tags(&a), ["Peak"]);

    // An agent now says Warm-up for a: Peak is the DJ's, kept.
    fact(&a, "warm-up");
    let plan = window.cli(&["--json", "project", "--mytags", "--dry-run"]).json();
    let kept: Vec<(String, String, String)> = plan["kept"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| (k["track"].as_str().unwrap().into(), k["tag"].as_str().unwrap().into(), k["reason"].as_str().unwrap().into()))
        .collect();
    assert_eq!(kept, [(a.clone(), MY_TAG_PEAK.to_owned(), "set_by_dj".to_owned())], "{plan}");
    // Warm-up, which no one set, is still the projection's to put on.
    assert!(plan["changes"].as_array().unwrap().iter().any(|c| c["track"] == a.as_str() && c["tag"] == MY_TAG_WARM_UP), "{plan}");
}
