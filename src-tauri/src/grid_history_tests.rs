//! wbx-64 review 2 and 7: the window's grid edits are steps of the one
//! shared Undo history, in the order the DJ made them, beside Energy and cue
//! edits; a tap run or a held nudge is one step; each written step is one
//! Ledger entry with its before and after beats.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::file_ledger_tests::{entries, Fixture};
use crate::grid::{GridAction, GridEdit, GridEditor, GridOptions};
use rbl_db::fixture;
use rbl_library::history::LibraryEdit;
use rbl_library::Touched;

fn track() -> String {
    fixture::track_id(1)
}

fn grid_edit(f: &Fixture, editor: &GridEditor, edit: GridEdit, transaction: Option<&str>) {
    let options = GridOptions::with_transaction(transaction.map(str::to_owned));
    crate::grid::guarded(
        &f.state,
        editor,
        &f.state.library().unwrap(),
        &f.location,
        &track(),
        GridAction::Edit {
            edit,
            from_ms: None,
        },
        &options,
    )
    .unwrap();
}

fn first_beat(f: &Fixture) -> u32 {
    rbl_anlz::Anlz::read(&f.dat).unwrap().beat_grid().unwrap()[0].time_ms
}

/// The beat nearest 520 ms: the fixture's beat at 500, however far it was
/// nudged, whatever beats a nudge fills in before it.
fn held_beat(f: &Fixture) -> u32 {
    let beats = rbl_anlz::Anlz::read(&f.dat).unwrap().beat_grid().unwrap();
    beats
        .iter()
        .map(|b| b.time_ms)
        .min_by_key(|t| t.abs_diff(520))
        .unwrap()
}

fn rating(f: &Fixture) -> i64 {
    let db = rbl_db::Library::open(f.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    db.connection()
        .query_row(
            "SELECT COALESCE(Rating, 0) FROM djmdContent WHERE ID=?1",
            [track()],
            |r| r.get(0),
        )
        .unwrap()
}

fn rate(f: &Fixture, stars: u8) {
    let id = track();
    f.state
        .core()
        .recorded_edit(
            &*f.state,
            Touched::Metadata(vec![id.clone()]),
            "Track Edit",
            |w| {
                w.set_rating_with_undo(&id, stars)
                    .map(|(_, edit)| ((), LibraryEdit::Track(vec![edit])))
            },
        )
        .unwrap();
}

fn hot_cues(f: &Fixture) -> Vec<String> {
    let db = rbl_db::Library::open(f.location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let mut statement = db
        .connection()
        .prepare("SELECT ID FROM djmdCue WHERE ContentID=?1 AND Kind NOT IN (0, 4) AND rb_local_deleted=0 ORDER BY ID")
        .unwrap();
    statement
        .query_map([track()], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn cue(f: &Fixture, edit: crate::cues::CueEdit, label: &'static str) {
    f.state
        .core()
        .recorded_edit(&*f.state, Touched::Cues(track()), label, |w| {
            let (_, revision) = crate::cues::apply_recorded(w, edit)
                .map_err(|e| rbl_db::DbError::WriteRefused(e.message))?;
            Ok(((), LibraryEdit::Cues(revision)))
        })
        .unwrap();
}

fn step(f: &Fixture, editor: &GridEditor, undo: bool) -> crate::AppResult<()> {
    crate::commands::step_history(&f.state, editor, undo).map(|_| ())
}

fn grid_entries(f: &Fixture) -> Vec<serde_json::Value> {
    entries(f)
        .into_iter()
        .filter(|e| e["kind"] == "window_grid")
        .collect()
}

#[test]
fn undo_takes_back_a_nudge_and_an_energy_edit_in_the_order_they_were_made() {
    let f = Fixture::new(3);
    let editor = GridEditor::at(f.dir.path());
    rate(&f, 4);
    grid_edit(&f, &editor, GridEdit::Nudge { ms: 100 }, None);
    assert_eq!((rating(&f), first_beat(&f)), (4, 600));

    // The latest step first, whatever the DJ was pointing at.
    step(&f, &editor, true).unwrap();
    assert_eq!((rating(&f), first_beat(&f)), (4, 500));
    step(&f, &editor, true).unwrap();
    assert_eq!((rating(&f), first_beat(&f)), (0, 500));

    step(&f, &editor, false).unwrap();
    assert_eq!((rating(&f), first_beat(&f)), (4, 500));
    step(&f, &editor, false).unwrap();
    assert_eq!((rating(&f), first_beat(&f)), (4, 600));
}

#[test]
fn undo_after_a_nudge_and_a_cleared_cue_brings_the_cue_back_first() {
    let f = Fixture::new(3);
    let editor = GridEditor::at(f.dir.path());
    cue(
        &f,
        crate::cues::CueEdit::Add {
            track: track(),
            kind: crate::cues::CueKind::Hot('A'),
            position_ms: 1000,
        },
        "Set Hot Cue",
    );
    let set = hot_cues(&f);
    assert_eq!(set.len(), 1);
    grid_edit(&f, &editor, GridEdit::Nudge { ms: -100 }, None);
    cue(
        &f,
        crate::cues::CueEdit::Delete {
            cue: set[0].clone(),
        },
        "Delete Cue",
    );
    assert!(hot_cues(&f).is_empty());

    step(&f, &editor, true).unwrap();
    assert_eq!(hot_cues(&f).len(), 1, "the cue comes back first");
    assert_eq!(first_beat(&f), 400, "the nudge stays");
    step(&f, &editor, true).unwrap();
    assert_eq!(first_beat(&f), 500);
    assert_eq!(hot_cues(&f).len(), 1);
}

#[test]
fn each_grid_step_is_one_ledger_entry_with_its_beats() {
    let f = Fixture::new(3);
    let editor = GridEditor::at(f.dir.path());
    grid_edit(&f, &editor, GridEdit::Nudge { ms: 100 }, None);
    step(&f, &editor, true).unwrap();
    step(&f, &editor, false).unwrap();
    let all = grid_entries(&f);
    assert_eq!(all.len(), 3, "{all:#?}");
    for (e, (action, before, after)) in
        all.iter()
            .zip([("edit", 500, 600), ("undo", 600, 500), ("redo", 500, 600)])
    {
        assert_eq!(e["action"], action);
        assert_eq!(e["command"]["args"]["before"][0][0], before);
        assert_eq!(e["command"]["args"]["after"][0][0], after);
    }
    // An undo names the write it takes back.
    assert_eq!(all[1]["reverses"], all[0]["id"]);
}

#[test]
fn a_held_nudge_is_one_undo_step_and_one_entry_a_write() {
    let f = Fixture::new(3);
    let editor = GridEditor::at(f.dir.path());
    rate(&f, 2);
    for _ in 0..4 {
        grid_edit(&f, &editor, GridEdit::Nudge { ms: 10 }, Some("hold-1"));
    }
    assert_eq!(held_beat(&f), 540);
    let written = grid_entries(&f);
    assert_eq!(written.len(), 4, "one Ledger entry per write: {written:#?}");
    // The later writes of the hold are part of its first.
    assert!(
        written[1..]
            .iter()
            .all(|e| e["part_of"] == written[0]["id"]),
        "{written:#?}"
    );
    step(&f, &editor, true).unwrap();
    assert_eq!(held_beat(&f), 500, "the whole hold in one Undo");
    assert_eq!(rating(&f), 2, "and nothing older");
    step(&f, &editor, false).unwrap();
    assert_eq!(held_beat(&f), 540, "Redo lands where the hold ended");
    // A second hold is a step of its own.
    grid_edit(&f, &editor, GridEdit::Nudge { ms: 10 }, Some("hold-2"));
    step(&f, &editor, true).unwrap();
    assert_eq!(held_beat(&f), 540);
}

#[test]
fn a_tap_run_is_one_undo_step_that_puts_back_every_beat() {
    let f = Fixture::new(3);
    let editor = GridEditor::at(f.dir.path());
    let original = rbl_anlz::Anlz::read(&f.dat).unwrap().beat_grid().unwrap();
    for bpm in [124.0, 125.5, 126.0] {
        grid_edit(
            &f,
            &editor,
            GridEdit::Tap {
                bpm,
                anchor_ms: 700,
            },
            Some("tap-run"),
        );
    }
    assert_ne!(
        rbl_anlz::Anlz::read(&f.dat).unwrap().beat_grid().unwrap(),
        original
    );
    step(&f, &editor, true).unwrap();
    assert_eq!(
        rbl_anlz::Anlz::read(&f.dat).unwrap().beat_grid().unwrap(),
        original
    );
    assert!(step(&f, &editor, true).is_err(), "nothing else to undo");
}

#[test]
fn a_grid_undo_under_library_protection_is_refused_and_kept() {
    let f = Fixture::new(3);
    let editor = GridEditor::at(f.dir.path());
    grid_edit(&f, &editor, GridEdit::Nudge { ms: 100 }, None);
    let bytes = std::fs::read(&f.dat).unwrap();
    f.state.core().set_protection(true).unwrap();
    assert!(step(&f, &editor, true).is_err());
    assert_eq!(std::fs::read(&f.dat).unwrap(), bytes, "no file changed");
    f.state.core().set_protection(false).unwrap();
    step(&f, &editor, true).unwrap();
    assert_eq!(first_beat(&f), 500, "the step was kept for later");
}

#[test]
fn the_grid_panel_undo_takes_only_its_own_track_s_step_from_the_top() {
    let f = Fixture::new(3);
    let editor = GridEditor::at(f.dir.path());
    grid_edit(&f, &editor, GridEdit::Nudge { ms: 100 }, None);
    rate(&f, 4);
    let undo = |f: &Fixture| {
        crate::grid::guarded(
            &f.state,
            &editor,
            &f.state.library().unwrap(),
            &f.location,
            &track(),
            GridAction::Undo,
            &GridOptions::default(),
        )
    };
    // An Energy edit is on top: the grid panel's Undo does not reach past it.
    assert!(undo(&f).is_err());
    assert_eq!((rating(&f), first_beat(&f)), (4, 600));
    step(&f, &editor, true).unwrap();
    undo(&f).unwrap();
    assert_eq!((rating(&f), first_beat(&f)), (0, 500));
}
