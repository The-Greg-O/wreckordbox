//! Query plans are part of the large-Library cost contract.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use wreckordbox_sidecar::Sidecar;
#[test]
fn track_queries_use_indexed_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(dir.path()).unwrap();
    let db = rusqlite::Connection::open(sidecar.dir().join("sidecar.db")).unwrap();
    for sql in [
        "SELECT * FROM attached_fact WHERE effective_track_id = 'track' AND kind = 'genre'",
        "SELECT EXISTS(SELECT 1 FROM attached_fact WHERE effective_track_id = 'track')",
        "UPDATE fact SET stale_at = 1 WHERE id IN (SELECT id FROM attached_fact WHERE effective_track_id = 'track')",
    ] {
        let plan = db.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap()
            .query_map([], |row| row.get::<_, String>(3)).unwrap()
            .collect::<rusqlite::Result<Vec<_>>>().unwrap().join("\n");
        assert!(plan.contains("fact_track_by_track"), "{sql}: {plan}");
        assert!(!plan.contains("SCAN fact"), "{plan}");
    }
}
