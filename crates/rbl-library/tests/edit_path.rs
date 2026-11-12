//! The edit path at the Library core's public interface, against a
//! throwaway Library. No window, no desktop framework.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rbl_library::{Change, ChangeKind, History, Host, LibraryCore, NoHost, Touched};

/// A core over a fresh fixture Library in `dir`.
fn core_in(dir: &std::path::Path) -> LibraryCore {
    let location = rbl_db::fixture::build(dir, rbl_db::fixture::Shape::default()).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (library, _) = rbl_index::load(&db).unwrap();
    let core = LibraryCore::new(dir.join("recovery"));
    core.set_library(&NoHost, library, false, db.schema().db_version, 0, location);
    core
}

fn rating_of(core: &LibraryCore, track: &str) -> u8 {
    let library = core.library().unwrap();
    library.rating[library.row_of(track).unwrap() as usize]
}

/// What a host is told, counted.
#[derive(Default)]
struct Recorder {
    recoveries: AtomicUsize,
    outside_history: AtomicUsize,
    history_changes: AtomicUsize,
    replaced: AtomicUsize,
    refuse_recovery: AtomicBool,
}

impl Host for Recorder {
    fn recover_interrupted_files(&self, _location: &rbl_db::LibraryLocation) -> Result<(), String> {
        self.recoveries.fetch_add(1, Ordering::SeqCst);
        if self.refuse_recovery.load(Ordering::SeqCst) { Err("recovery refused".to_owned()) } else { Ok(()) }
    }
    fn wrote_outside_history(&self) {
        self.outside_history.fetch_add(1, Ordering::SeqCst);
    }
    fn history_changed(&self) {
        self.history_changes.fetch_add(1, Ordering::SeqCst);
    }
    fn library_replaced(&self, _another_library: bool) {
        self.replaced.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_rating_edit_is_written_refreshed_and_given_a_new_generation() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);
    let before = core.library().unwrap();
    let generation = core.generation();
    let was = rating_of(&core, &track);
    let stars = if was == 4 { 3 } else { 4 };

    let (_, change) = core
        .edit(&NoHost, History::Outside, Touched::Metadata(vec![track.clone()]), |w| w.set_rating(&track, stars))
        .unwrap();

    assert_eq!(change, Change { kind: ChangeKind::Library, generation: generation + 1 });
    assert_eq!(core.generation(), generation + 1);
    assert_eq!(rating_of(&core, &track), stars, "the index shows the edit");
    assert_eq!(before.rating[before.row_of(&track).unwrap() as usize], was, "a reader's snapshot is untouched");
    let (persisted, _) = rbl_index::load(&core.open_read_only().unwrap()).unwrap();
    assert_eq!(persisted.rating[persisted.row_of(&track).unwrap() as usize], stars, "and so does the Library on disk");
}

#[test]
fn a_refused_edit_changes_neither_the_index_nor_the_generation() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);
    let generation = core.generation();
    let was = rating_of(&core, &track);

    // Six stars is not a rating.
    let result = core.edit(&NoHost, History::Outside, Touched::Metadata(vec![track.clone()]), |w| w.set_rating(&track, 6));

    assert!(result.is_err());
    assert_eq!(core.generation(), generation);
    assert_eq!(rating_of(&core, &track), was);
}

#[test]
fn a_tag_list_edit_keeps_the_generation_and_every_other_view() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);
    let spec = |source| rbl_index::ViewSpec {
        source,
        sort: rbl_index::SortColumn::TrackNo,
        descending: false,
        query: String::new(),
        filter: rbl_index::TrackFilter::default(),
    };
    let (collection, ..) = core.open_view_scoped(&spec(rbl_index::TrackSource::Collection), rbl_index::SearchField::All).unwrap();
    let (tag_list, ..) = core.open_view_scoped(&spec(rbl_index::TrackSource::TagList), rbl_index::SearchField::All).unwrap();
    let generation = core.generation();

    let (_, change) = core
        .edit(&NoHost, History::Outside, Touched::TagList, |w| w.tag_list_add(std::slice::from_ref(&track)))
        .unwrap();

    assert_eq!(change, Change { kind: ChangeKind::TagList, generation });
    assert!(core.view(collection).is_some(), "the collection view stays open");
    assert!(core.view(tag_list).is_none(), "the Tag List view is stale");

    // A playlist edit, by contrast, closes every view under a new generation.
    let (_, change) = core
        .edit(&NoHost, History::Outside, Touched::Playlists, |w| w.create_playlist("Warm up", "root"))
        .unwrap();
    assert_eq!(change, Change { kind: ChangeKind::Library, generation: generation + 1 });
    assert!(core.view(collection).is_none());
}

#[test]
fn the_host_hears_of_writes_outside_the_history_only() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let host = Recorder::default();
    let track = rbl_db::fixture::track_id(1);
    let touched = || Touched::Metadata(vec![track.clone()]);
    let stars = if rating_of(&core, &track) == 2 { 1 } else { 2 };

    core.edit(&host, History::Caller, touched(), |w| w.set_rating(&track, stars)).unwrap();
    assert_eq!(host.outside_history.load(Ordering::SeqCst), 0, "the caller keeps the history for this one");

    core.edit(&host, History::Outside, touched(), |w| w.set_rating(&track, stars + 1)).unwrap();
    assert_eq!(host.outside_history.load(Ordering::SeqCst), 1);

    // A write that changed no row ends nothing.
    core.write_then(&host, History::Outside, |_| Ok(()), |_, ()| Ok(())).unwrap();
    assert_eq!(host.outside_history.load(Ordering::SeqCst), 1);

    // A failed edit that had already changed rows still counts.
    let failed: Result<(), _> = core.write_then(
        &host,
        History::Outside,
        |w| {
            w.set_rating(&track, stars)?;
            w.set_rating(&track, 6).map(|_| ())
        },
        |_, ()| Ok(()),
    );
    assert!(failed.is_err());
    assert_eq!(host.outside_history.load(Ordering::SeqCst), 2);

    assert_eq!(host.recoveries.load(Ordering::SeqCst), 4, "every write asks the host to recover first");
}

#[test]
fn a_write_is_refused_when_the_host_cannot_recover_and_skips_recovery_during_an_analysis_write() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let host = Recorder::default();
    host.refuse_recovery.store(true, Ordering::SeqCst);
    let track = rbl_db::fixture::track_id(1);
    let was = rating_of(&core, &track);
    let stars = if was == 2 { 1 } else { 2 };

    let refused = core.edit(&host, History::Outside, Touched::Metadata(vec![track.clone()]), |w| w.set_rating(&track, stars));
    assert!(matches!(refused, Err(rbl_db::DbError::WriteRefused(reason)) if reason == "recovery refused"));
    assert_eq!(rating_of(&core, &track), was);

    // An analysis write publishes its own journal: recovery is left alone.
    let _files = core.analysis_write().lock();
    core.edit(&host, History::Outside, Touched::Metadata(vec![track.clone()]), |w| w.set_rating(&track, stars)).unwrap();
    assert_eq!(host.recoveries.load(Ordering::SeqCst), 1);
    assert_eq!(rating_of(&core, &track), stars);
}

#[test]
fn a_write_is_refused_while_a_restore_is_unfinished() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);
    // What RBXport Restore leaves behind while it is mid-way.
    std::fs::create_dir_all(core.recovery_dir()).unwrap();
    std::fs::write(core.recovery_dir().join(rbl_backup::journal::NAME), b"{}").unwrap();
    assert!(rbl_backup::journal::pending(core.recovery_dir()));

    let refused = core.edit(&NoHost, History::Outside, Touched::Metadata(vec![track.clone()]), |w| w.set_rating(&track, 1));
    assert!(matches!(refused, Err(rbl_db::DbError::WriteRefused(reason)) if reason.contains("restore is unfinished")));
    assert!(matches!(core.open_read_only(), Err(rbl_db::DbError::WriteRefused(_))));
}

#[test]
fn a_track_edit_reads_the_whole_index_again_and_tells_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let host = Recorder::default();
    let track = rbl_db::fixture::track_id(1);
    let generation = core.generation();

    let (_, change) = core
        .edit(&host, History::Outside, Touched::Tracks, |w| w.set_field(&track, rbl_db::write::TrackField::Title, "Renamed"))
        .unwrap();

    assert_eq!(change, Change { kind: ChangeKind::Library, generation: generation + 1 });
    assert_eq!(host.replaced.load(Ordering::SeqCst), 1);
    let library = core.library().unwrap();
    assert_eq!(library.title.get(library.row_of(&track).unwrap() as usize), "Renamed");
}

#[test]
fn nothing_can_be_written_before_a_library_is_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let core = LibraryCore::new(dir.path().join("recovery"));
    let result = core.edit(&NoHost, History::Outside, Touched::Playlists, |_| Ok(()));
    assert!(matches!(result, Err(rbl_db::DbError::NotInstalled(reason)) if reason == rbl_library::NOT_LOADED));
}

/// The guard is `rbl-db`'s, taken on every write the core opens: under the
/// test environment variable a Library detected as the real install is
/// never opened for writing. The Library here is a fixture that only says
/// it is the real one, so nothing real is at stake either way.
#[test]
fn a_detected_real_library_is_not_written_under_the_test_variable() {
    std::env::set_var(rbl_db::TEST_ENV, "1");
    let dir = tempfile::tempdir().unwrap();
    let mut location = rbl_db::fixture::build(dir.path(), rbl_db::fixture::Shape::default()).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (library, _) = rbl_index::load(&db).unwrap();
    location.is_real_install = true;
    let core = LibraryCore::new(dir.path().join("recovery"));
    core.set_library(&NoHost, library, false, db.schema().db_version, 0, location);
    let track = rbl_db::fixture::track_id(1);
    let was = rating_of(&core, &track);
    let generation = core.generation();

    let refused = core.edit(&NoHost, History::Outside, Touched::Metadata(vec![track.clone()]), |w| w.set_rating(&track, (was + 1) % 6));

    assert!(matches!(refused, Err(rbl_db::DbError::WriteRefused(reason)) if reason.contains("RBXPORT_TEST")));
    assert_eq!(core.generation(), generation);
    let (persisted, _) = rbl_index::load(&core.open_read_only().unwrap()).unwrap();
    assert_eq!(persisted.rating[persisted.row_of(&track).unwrap() as usize], was);
}

#[test]
fn a_watcher_hears_every_change_in_order_until_it_says_stop() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);
    let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let keep = std::sync::Arc::new(AtomicBool::new(true));
    {
        let (heard, keep) = (std::sync::Arc::clone(&heard), std::sync::Arc::clone(&keep));
        core.watch(move |change| {
            heard.lock().unwrap().push(change);
            keep.load(Ordering::SeqCst)
        });
    }

    let (_, rated) = core
        .edit(&NoHost, History::Outside, Touched::Metadata(vec![track.clone()]), |w| w.set_rating(&track, 4))
        .unwrap();
    let (_, tagged) = core
        .edit(&NoHost, History::Outside, Touched::TagList, |w| w.tag_list_add(std::slice::from_ref(&track)))
        .unwrap();
    // A reload by whoever holds the core is a change like any other.
    let reloaded = core.invalidate_views();

    assert_eq!(
        *heard.lock().unwrap(),
        [rated, tagged, Change { kind: ChangeKind::Library, generation: reloaded }]
    );
    assert_eq!(tagged.kind, ChangeKind::TagList);

    keep.store(false, Ordering::SeqCst);
    core.invalidate_views();
    core.invalidate_views();
    assert_eq!(heard.lock().unwrap().len(), 4, "a watcher that said stop is not told again");
}

#[test]
fn writes_are_refused_with_the_holders_reason_until_it_allows_them_again() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let track = rbl_db::fixture::track_id(1);
    let generation = core.generation();

    core.refuse_writes(Some("Another wreckordbox window has this Library.".to_owned()));
    let refused = core
        .edit(&NoHost, History::Outside, Touched::Metadata(vec![track.clone()]), |w| w.set_rating(&track, 5))
        .unwrap_err();

    assert!(matches!(&refused, rbl_db::DbError::WriteRefused(why) if why == "Another wreckordbox window has this Library."), "{refused}");
    assert_eq!(core.generation(), generation);
    assert_eq!(core.write_refusal().as_deref(), Some("Another wreckordbox window has this Library."));

    core.refuse_writes(None);
    core.edit(&NoHost, History::Outside, Touched::Metadata(vec![track.clone()]), |w| w.set_rating(&track, 5)).unwrap();
    assert_eq!(rating_of(&core, &track), 5);
}

#[test]
fn a_recorded_edit_is_undone_and_redone_and_an_outside_write_ends_the_redo_branch() {
    use rbl_library::history::{LibraryEdit, StepError, TRACK_EDIT};

    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let host = Recorder::default();
    let (track, other) = (rbl_db::fixture::track_id(1), rbl_db::fixture::track_id(2));
    let rate = |stars: u8| {
        core.recorded_edit(&host, Touched::Metadata(vec![track.clone()]), TRACK_EDIT, |w| {
            let (_, edit) = w.set_rating_with_undo(&track, stars)?;
            Ok(((), LibraryEdit::Track(vec![edit])))
        })
        .unwrap()
    };

    rate(4);
    assert_eq!(core.history().lock().undo_label(), Some(TRACK_EDIT));
    assert_eq!(host.history_changes.load(Ordering::SeqCst), 1, "the host is told an entry was recorded");
    // The stars it already has: nothing to record, nobody told.
    rate(4);
    assert_eq!(host.history_changes.load(Ordering::SeqCst), 1);

    let undone = core.step_history(&host, true).unwrap();
    assert_eq!((rating_of(&core, &track), undone.label, undone.generation), (0, TRACK_EDIT, core.generation()));
    assert!(matches!(core.step_history(&host, true), Err(StepError::Nothing)));
    core.step_history(&host, false).unwrap();
    assert_eq!(rating_of(&core, &track), 4);
    assert_eq!(host.history_changes.load(Ordering::SeqCst), 1, "a step is the caller's own to announce");

    // A write the history does not record ends the Redo branch, and only
    // then is the host told the history changed.
    core.step_history(&host, true).unwrap();
    core.edit(&host, History::Outside, Touched::Metadata(vec![other.clone()]), |w| w.set_rating(&other, 1)).unwrap();
    assert_eq!(core.history().lock().redo_label(), None);
    assert_eq!(host.history_changes.load(Ordering::SeqCst), 2);
    core.edit(&host, History::Outside, Touched::Metadata(vec![other.clone()]), |w| w.set_rating(&other, 2)).unwrap();
    assert_eq!(host.history_changes.load(Ordering::SeqCst), 2, "no Redo branch to end");
}

#[test]
fn undo_refuses_a_value_changed_outside_the_history_and_drops_only_that_entry() {
    use rbl_library::history::{LibraryEdit, StepError, TRACK_EDIT};

    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let (first, second) = (rbl_db::fixture::track_id(1), rbl_db::fixture::track_id(2));
    for track in [&first, &second] {
        core.recorded_edit(&NoHost, Touched::Metadata(vec![track.clone()]), TRACK_EDIT, |w| {
            let (_, edit) = w.set_rating_with_undo(track, 4)?;
            Ok(((), LibraryEdit::Track(vec![edit])))
        })
        .unwrap();
    }
    core.edit(&NoHost, History::Outside, Touched::Metadata(vec![second.clone()]), |w| w.set_rating(&second, 1)).unwrap();

    let refused = core.step_history(&NoHost, true).unwrap_err();

    assert!(matches!(refused, StepError::Conflict { label: TRACK_EDIT, .. }), "{refused:?}");
    assert_eq!(rating_of(&core, &second), 1, "the newer value stays");
    core.step_history(&NoHost, true).unwrap();
    assert_eq!(rating_of(&core, &first), 0, "the entry before it still runs");
}
