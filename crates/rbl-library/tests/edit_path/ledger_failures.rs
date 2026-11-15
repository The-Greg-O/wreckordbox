use super::*;
use rbl_library::history::{LibraryEdit, TRACK_EDIT};

#[test]
fn rollback_is_not_reported_as_a_committed_write() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let tape = std::sync::Arc::new(LedgerTape::default());
    core.history().lock().keep_ledger(Some(tape.clone()));
    let track = rbl_db::fixture::track_id(1);
    let before = rating_of(&core, &track);
    let result: Result<(), _> = core.write_then(
        &NoHost,
        History::Outside,
        |w| {
            w.library()
                .connection()
                .execute_batch("BEGIN IMMEDIATE; UPDATE djmdContent SET Rating=5; ROLLBACK;")?;
            Err(rbl_db::DbError::WriteRefused("fixture rollback".into()))
        },
        |_, v| Ok(v),
    );
    assert!(result.is_err());
    assert_eq!(*tape.written.lock(), vec![false]);
    assert_eq!(rating_of(&core, &track), before);
    assert!(tape.notes.lock().is_empty());
}

#[test]
fn a_committed_prefix_survives_a_later_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let tape = std::sync::Arc::new(LedgerTape::default());
    core.history().lock().keep_ledger(Some(tape.clone()));
    let track = rbl_db::fixture::track_id(1);
    let result: Result<(), _> = core.write_then(
        &NoHost,
        History::Outside,
        |w| {
            w.set_rating(&track, 4)?;
            w.library()
                .connection()
                .execute_batch("BEGIN; UPDATE djmdContent SET Rating=5; ROLLBACK;")?;
            Err(rbl_db::DbError::WriteRefused("fixture suffix".into()))
        },
        |_, v| Ok(v),
    );
    assert!(result.is_err());
    assert_eq!(*tape.written.lock(), vec![true]);
    assert_eq!(
        core.open_read_only()
            .unwrap()
            .connection()
            .query_row(
                "SELECT Rating FROM djmdContent WHERE ID=?1",
                [&track],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        4
    );
}

#[test]
fn permanent_refresh_failure_records_and_invalidates_history() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_in(dir.path());
    let tape = std::sync::Arc::new(LedgerTape::default());
    core.history().lock().keep_ledger(Some(tape.clone()));
    let track = rbl_db::fixture::track_id(1);
    core.recorded_edit(
        &NoHost,
        Touched::Metadata(vec![track.clone()]),
        TRACK_EDIT,
        |w| {
            let (_, edit) = w.set_rating_with_undo(&track, 3)?;
            Ok(((), LibraryEdit::Track(vec![edit])))
        },
    )
    .unwrap();
    let result = core.permanent_edit(&NoHost, Touched::Metadata(vec![track.clone()]), |w| {
        w.set_rating(&track, 4)?;
        w.library()
            .connection()
            .execute_batch("ALTER TABLE djmdContent RENAME TO ContentGone")?;
        Ok(())
    });
    assert!(result.is_err());
    assert!(
        core.history().lock().undo_label().is_none(),
        "an accepted permanent edit must invalidate Undo even when refresh fails"
    );
    assert_eq!(
        *tape.notes.lock(),
        vec![(rbl_library::ledger::Unkept::Permanent, None)]
    );
}
