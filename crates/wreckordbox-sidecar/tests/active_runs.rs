#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

#[test]
fn unfinished_runs_use_a_partial_index_after_migration() {
    let dir = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("sidecar.db")).unwrap();
    db.execute_batch(include_str!("fixtures/sidecar-v1.sql"))
        .unwrap();
    db.execute_batch(include_str!("fixtures/sidecar-v2.sql"))
        .unwrap();
    drop(db);
    let sidecar = wreckordbox_sidecar::Sidecar::open(dir.path()).unwrap();
    let source = wreckordbox_sidecar::Source::new(wreckordbox_sidecar::SourceKind::Tool, "test");
    let active = sidecar.begin_run(&source, "Active").unwrap();
    let finished = sidecar.begin_run(&source, "Finished").unwrap();
    sidecar.finish_run(finished, "done").unwrap();
    let db = rusqlite::Connection::open(dir.path().join("sidecar.db")).unwrap();
    let plan: String = db.query_row("EXPLAIN QUERY PLAN SELECT id, description FROM run WHERE finished_at IS NULL ORDER BY id", [], |row| row.get(3)).unwrap();
    assert!(plan.contains("USING INDEX run_unfinished"), "{plan}");
    assert_eq!(
        sidecar
            .active_runs()
            .unwrap()
            .iter()
            .map(|run| run.id)
            .collect::<Vec<_>>(),
        vec![active.0]
    );
}
