use super::*;
use serde_json::json;

#[test]
fn cue_conversion_records_each_slice_and_undo_reverses_the_group() {
    let window = window();
    window.load();
    window.host().unwrap();
    let track = track_id(1);
    for position in [1000, 2000] {
        run(rbxport_lib::cues::add_cue(
            window.app.handle().clone(),
            window.state(),
            track.clone(),
            rbxport_lib::cues::CueKind::Memory,
            position,
        ))
        .unwrap();
    }
    let before = window.cli(&["--json", "runs", "list"]).json();
    let id = &before["runs"][0]["id"];
    let count = ledger_entries(&window, id).len();
    let made = run(rbxport_lib::cues::convert_memory_cues_to_hot(
        window.app.handle().clone(),
        window.state(),
        track.clone(),
    ))
    .unwrap();
    assert_eq!(made, 2);
    let entries = ledger_entries(&window, id);
    let converted = &entries[count..];
    assert_eq!(converted.len(), 2);
    assert!(
        converted.iter().all(|e| e["kind"] == "cues"),
        "{converted:?}"
    );
    assert_eq!(converted[1]["part_of"], converted[0]["id"]);
    run(commands::undo_edit(
        window.app.handle().clone(),
        window.state(),
    ))
    .unwrap();
    let after = ledger_entries(&window, id);
    assert_eq!(after.last().unwrap()["reverses"], converted[0]["id"]);
    let library = window.state().library().unwrap();
    assert!(library
        .cues_of(library.row_of(&track).unwrap())
        .iter()
        .all(|c| c.hot_letter().is_none()));
    window.protect(true);
    let refused = run(rbxport_lib::cues::convert_memory_cues_to_hot(
        window.app.handle().clone(),
        window.state(),
        track,
    ));
    assert!(refused.is_err());
    assert_eq!(ledger_entries(&window, id), after);
}

#[test]
fn spill_commands_are_available_through_the_cli_without_creating_a_sidecar() {
    let window = window();
    window.host().unwrap();
    assert_eq!(
        window.cli(&["--json", "runs", "spills", "list"]).json()["total"],
        0
    );
    assert!(!window
        .dir
        .path()
        .join("library/wreckordbox-sidecar")
        .exists());
    assert!(!window.cli(&["runs", "spills", "show", "../outside"]).ok);
    assert!(
        !window
            .cli(&[
                "runs",
                "spills",
                "discard",
                "missing",
                "--source",
                "agent:reviewer",
                "--reason",
                "fixture"
            ])
            .ok
    );
    assert_eq!(
        window.cli(&["--json", "runs", "list"]).json()["total"],
        json!(0)
    );
}
