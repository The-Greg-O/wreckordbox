#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use tauri::Manager;
#[derive(Default)]
struct Tape(parking_lot::Mutex<Vec<(rbl_library::ledger::Unkept, Option<String>)>>);
impl rbl_library::ledger::Ledger for Tape {
    fn begin(&self) -> Result<(), String> {
        Ok(())
    }
    fn written(&self, _wrote: bool) {}
    fn record(
        &self,
        _step: rbl_library::ledger::Step,
        _edit: &LibraryEdit,
        _label: &str,
        _link: rbl_library::ledger::Link,
    ) -> Option<i64> {
        None
    }
    fn note(&self, kind: rbl_library::ledger::Unkept, name: Option<&str>) {
        self.0.lock().push((kind, name.map(str::to_owned)));
    }
}
fn check(permanent: bool) {
    let dir = tempfile::tempdir().unwrap();
    let location = rbl_db::fixture::build(dir.path(), rbl_db::fixture::Shape::default()).unwrap();
    let db = rbl_db::Library::open(location.clone(), rbl_db::OpenMode::ReadOnly).unwrap();
    let (index, _) = rbl_index::load(&db).unwrap();
    let state = Arc::new(AppState::with_backups(dir.path().join("recovery")));
    state.set_library(index, false, db.schema().db_version, 0, location);
    state.core().set_protection(false).unwrap();
    let app = tauri::test::mock_app();
    app.manage(state.clone());
    let track = rbl_db::fixture::track_id(1);
    tauri::async_runtime::block_on(set_track_rating(
        app.handle().clone(),
        app.state(),
        vec![track.clone()],
        3,
    ))
    .unwrap();
    if !permanent {
        tauri::async_runtime::block_on(undo_edit(app.handle().clone(), app.state())).unwrap();
    }
    let tape = Arc::new(Tape::default());
    state.edit_history().lock().keep_ledger(Some(tape.clone()));
    let action = move |w: &mut rbl_db::write::Writer| {
        w.set_rating(&track, 4)?;
        w.library()
            .connection()
            .execute_batch("ALTER TABLE djmdContent RENAME TO ContentGone")?;
        Ok(())
    };
    let touched = Touched::Tracks;
    let result = if permanent {
        tauri::async_runtime::block_on(permanent_edit(
            app.handle().clone(),
            app.state(),
            "fixture permanent",
            touched,
            action,
        ))
    } else {
        tauri::async_runtime::block_on(edit(
            app.handle().clone(),
            app.state(),
            "fixture unrecorded",
            touched,
            action,
        ))
    };
    assert!(result.is_err());
    let expected = if permanent {
        rbl_library::ledger::Unkept::Permanent
    } else {
        rbl_library::ledger::Unkept::Unrecorded
    };
    assert_eq!(
        tape.0.lock().as_slice(),
        &[(
            expected,
            Some(
                if permanent {
                    "fixture permanent"
                } else {
                    "fixture unrecorded"
                }
                .into()
            )
        )]
    );
    assert!(state.edit_history().lock().redo_label().is_none());
    if permanent {
        assert!(state.edit_history().lock().undo_label().is_none());
    }
    assert_eq!(
        db.connection()
            .query_row(
                "SELECT Rating FROM ContentGone WHERE ID=?1",
                [rbl_db::fixture::track_id(1)],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        4
    );
}
#[test]
fn unrecorded_refresh_failure_records_before_returning_error() {
    check(false);
}
#[test]
fn permanent_refresh_failure_records_before_returning_error() {
    check(true);
}
