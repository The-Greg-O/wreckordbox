use super::*;
#[test]
fn refused_and_rolled_back_writes_leave_no_durable_intent() {
    let f = Fixture::new(1);
    for rollback in [false, true] {
        let result: Result<(), _> = f.core.write_then(
            &NoHost,
            History::Outside,
            |w| {
                if rollback {
                    w.library()
                        .connection()
                        .execute_batch("BEGIN; UPDATE djmdContent SET Rating=5; ROLLBACK")?;
                }
                Err(rbl_db::DbError::WriteRefused(
                    "known fixture refusal".into(),
                ))
            },
            |_, v| Ok(v),
        );
        assert!(result.is_err());
        let db = rusqlite::Connection::open_with_flags(
            f.dir.path().join("library/wreckordbox-sidecar/sidecar.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM ledger_intent", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "known no-write must be durably settled immediately"
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM ledger", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(f.service.call("runs.list", json!({})).unwrap()["total"], 0);
    }
}
