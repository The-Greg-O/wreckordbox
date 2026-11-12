//! Upgrade a version 1 Sidecar without losing Fact provenance.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;

use wreckordbox_sidecar::{attachments::IdentifyingFields, fingerprint_file, Sidecar};

#[test]
fn old_facts_can_reattach_after_migration_but_known_stale_audio_is_not_an_identity() {
    for (version, stale) in [1, 2, 3, 4]
        .into_iter()
        .flat_map(|version| [false, true].map(|stale| (version, stale)))
    {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("fixture.wav");
        common::write_audio(&audio, 100, 1, None);
        let fingerprint = fingerprint_file(&audio).unwrap();
        let folder = dir.path().join("sidecar");
        std::fs::create_dir_all(&folder).unwrap();
        let db = rusqlite::Connection::open(folder.join("sidecar.db")).unwrap();
        db.execute_batch(include_str!("fixtures/sidecar-v1.sql"))
            .unwrap();
        if version >= 2 {
            db.execute_batch(include_str!("fixtures/sidecar-v2.sql"))
                .unwrap();
        }
        if version >= 3 {
            db.execute_batch(include_str!("fixtures/sidecar-v3.sql"))
                .unwrap();
        }
        if version >= 4 {
            db.execute_batch(include_str!("fixtures/sidecar-v4.sql"))
                .unwrap();
        }
        // Add multiple Facts using only the historical schema, never today's writer.
        db.execute("INSERT INTO fact (track_id,fingerprint,kind,value,source_kind,source_name,confidence,run_id,recorded_at,stale_at) VALUES ('removed',?1,'intensity','{\"summary\":0.7}','tool','fixture',1,7,1794400003000,?2)", rusqlite::params![fingerprint.as_str(), stale.then_some(1794400004000_i64)]).unwrap();
        let original_id = db.last_insert_rowid();
        db.execute("INSERT INTO fact (track_id,fingerprint,kind,value,source_kind,source_name,confidence,run_id,recorded_at) VALUES ('removed',NULL,'genre','\"Techno\"','dj','dj',1,7,1794400003001)", []).unwrap();
        let judgement_id = db.last_insert_rowid();
        let before: String = db.query_row("SELECT group_concat(quote(id)||quote(track_id)||quote(fingerprint)||quote(value)||quote(run_id)||quote(stale_at)) FROM fact", [], |r| r.get(0)).unwrap();
        drop(db);
        let sidecar = Sidecar::open(&folder).unwrap();
        let result = sidecar
            .observe_audio(
                "readded",
                Some(&fingerprint),
                &IdentifyingFields::default(),
                |_| false,
            )
            .unwrap();
        if stale {
            assert!(result.reattached.is_empty());
            assert!(sidecar.fact(original_id).unwrap().unwrap().is_stale());
        } else {
            assert_eq!(result.reattached[0].fact_ids, [original_id, judgement_id]);
            let facts = sidecar.facts_of_track("readded").unwrap();
            assert_eq!(facts.len(), 2);
            assert_eq!(facts[1].run.0, 7);
            assert_eq!(facts[1].source.name, "fixture");
            drop(sidecar);
            assert_eq!(
                Sidecar::open(&folder)
                    .unwrap()
                    .facts_of_track("readded")
                    .unwrap(),
                facts
            );
        }
        let db = rusqlite::Connection::open(folder.join("sidecar.db")).unwrap();
        let after: String = db.query_row("SELECT group_concat(quote(id)||quote(track_id)||quote(fingerprint)||quote(value)||quote(run_id)||quote(stale_at)) FROM fact", [], |r| r.get(0)).unwrap();
        assert_eq!(
            before, after,
            "migration/attachment must preserve provenance"
        );
    }
}

#[test]
fn failed_attachment_migration_keeps_the_frozen_store_retryable() {
    let dir = tempfile::tempdir().unwrap();
    let db = rusqlite::Connection::open(dir.path().join("sidecar.db")).unwrap();
    db.execute_batch(include_str!("fixtures/sidecar-v1.sql"))
        .unwrap();
    db.execute_batch(include_str!("fixtures/sidecar-v2.sql"))
        .unwrap();
    db.execute_batch(include_str!("fixtures/sidecar-v3.sql"))
        .unwrap();
    db.execute_batch(include_str!("fixtures/sidecar-v4.sql"))
        .unwrap();
    db.execute_batch("CREATE TABLE attachment_review (obstruction TEXT);")
        .unwrap();
    assert!(Sidecar::open(dir.path()).is_err());
    let version: String = db
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(version, "4");
    let partial: i64 = db.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('track_identity','fact_track','fact_initial_track')", [], |r| r.get(0)).unwrap();
    assert_eq!(partial, 0);
    db.execute_batch("DROP TABLE attachment_review;").unwrap();
    assert_eq!(
        Sidecar::open(dir.path())
            .unwrap()
            .facts_of_track("10003")
            .unwrap()
            .len(),
        2
    );
}
