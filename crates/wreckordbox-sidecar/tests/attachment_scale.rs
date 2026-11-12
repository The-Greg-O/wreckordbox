//! Reproducible costs on the crate's bundled SQLite, 200,000 Facts / 20,000 tracks.
#![allow(
    clippy::pedantic,
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;
use rusqlite::{params, Connection};
use serde_json::json;
use std::time::Instant;
use wreckordbox_sidecar::{
    attachments::IdentifyingFields, fingerprint_file, Kind, NewFact, Sidecar, Source, SourceKind,
};

fn timed(n: u32, mut op: impl FnMut()) -> f64 {
    let started = Instant::now();
    for _ in 0..n {
        op();
    }
    started.elapsed().as_secs_f64() * 1000.0 / f64::from(n)
}

#[test]
#[ignore = "explicit large-store benchmark"]
fn facts_at_library_scale() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("fixture.wav");
    common::write_audio(&audio, 100, 1, None);
    let fingerprint = fingerprint_file(&audio).unwrap();
    let sidecar = Sidecar::open(&dir.path().join("sidecar")).unwrap();
    let source = Source::new(SourceKind::Tool, "bench");
    let run = sidecar.begin_run(&source, "Large-store benchmark").unwrap();
    let db = Connection::open(sidecar.dir().join("sidecar.db")).unwrap();
    db.execute_batch("INSERT INTO kind VALUES ('intensity','measurement');
        WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<199999)
        INSERT INTO fact(track_id,fingerprint,kind,value,source_kind,source_name,confidence,run_id,recorded_at)
        SELECT printf('track-%d',x/10),printf('audio-%d',x/10),'intensity','{}','tool','bench',1,1,x FROM n;
        INSERT INTO track_identity SELECT track_id,fingerprint,'','','',0 FROM fact GROUP BY track_id;
        CREATE VIEW reviewed_scan AS SELECT fact.*, COALESCE((SELECT c.to_track FROM fact_attachment a
        JOIN attachment_change c ON c.id=a.change_id WHERE a.fact_id=fact.id ORDER BY c.id DESC LIMIT 1), fact.track_id)
        AS effective_track_id FROM fact;").unwrap();
    db.execute(
        "UPDATE track_identity SET fingerprint=?1 WHERE track_id='track-10000'",
        [fingerprint.as_str()],
    )
    .unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM fact", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 200_000);
    let mut old_read = db.prepare("SELECT id,value FROM reviewed_scan WHERE effective_track_id='track-10000' AND kind='intensity'").unwrap();
    let old_read_ms = timed(100, || {
        let rows = old_read
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows.len(), 10);
    });
    let new_read_ms = timed(100, || {
        assert_eq!(sidecar.facts("track-10000", "intensity").unwrap().len(), 10);
    });
    let old_supersede_ms = timed(100, || {
        db.execute("UPDATE fact SET superseded_at=1 WHERE id IN (SELECT id FROM reviewed_scan WHERE effective_track_id='absent') AND kind='intensity'", []).unwrap();
    });
    let new_record_ms = timed(100, || {
        sidecar
            .record(&NewFact {
                track_id: "new",
                fingerprint: Some(&fingerprint),
                kind: Kind::measurement("intensity"),
                value: json!({}),
                source: &source,
                confidence: 1.0,
                run,
            })
            .unwrap();
    });
    let old_stale_ms = timed(100, || {
        db.execute("UPDATE fact SET stale_at=1 WHERE id IN (SELECT id FROM reviewed_scan WHERE effective_track_id='absent') AND fingerprint <> 'current'", []).unwrap();
    });
    let new_stale_ms = timed(100, || {
        sidecar.mark_stale("track-10000", &fingerprint).unwrap();
    });
    let old_occupied_ms = timed(100, || {
        assert!(!db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM reviewed_scan WHERE effective_track_id='absent')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    });
    let new_occupied_ms = timed(100, || {
        assert!(!db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM attached_fact WHERE effective_track_id='absent')",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap());
    });
    let old_identity_ms = timed(20, || {
        let mut q = db.prepare("SELECT track_id FROM track_identity WHERE track_id IN (SELECT effective_track_id FROM reviewed_scan)").unwrap();
        assert_eq!(
            q.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .len(),
            20_000
        );
    });
    let mut visited = 0;
    let new_observe_ms = timed(100, || {
        let seen = std::cell::Cell::new(0);
        sidecar
            .observe_audio(
                "candidate",
                Some(&fingerprint),
                &IdentifyingFields::default(),
                |_| {
                    seen.set(seen.get() + 1);
                    true
                },
            )
            .unwrap();
        visited = seen.get();
    });
    assert_eq!(
        visited, 1,
        "only fingerprint candidates reach the Library-presence check"
    );
    let sqlite: String = db
        .query_row("SELECT sqlite_version()", params![], |r| r.get(0))
        .unwrap();
    println!("SQLite {sqlite}, {count} Facts; milliseconds/op (100 iterations, identities 20):");
    println!("read old={old_read_ms:.4} new(API, full Facts)={new_read_ms:.4}");
    println!("supersede old={old_supersede_ms:.4} new(full record)={new_record_ms:.4}");
    println!("stale old={old_stale_ms:.4} new(API)={new_stale_ms:.4}");
    println!("occupied old={old_occupied_ms:.4} new={new_occupied_ms:.4}");
    println!("identities old={old_identity_ms:.4} new(full observation)={new_observe_ms:.4}; candidates={visited}");
}
