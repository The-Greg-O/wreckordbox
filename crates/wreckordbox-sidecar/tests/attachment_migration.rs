//! Upgrade a version 1 Sidecar without losing Fact provenance.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]
mod common;

use serde_json::json;
use wreckordbox_sidecar::{
    attachments::IdentifyingFields, fingerprint_file, Kind, NewFact, Sidecar, Source, SourceKind,
};

#[test]
fn old_facts_can_reattach_after_migration_but_known_stale_audio_is_not_an_identity() {
    for (version, stale) in [1, 2, 3]
        .into_iter()
        .flat_map(|version| [false, true].map(|stale| (version, stale)))
    {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("fixture.wav");
        common::write_audio(&audio, 100, 1, None);
        let fingerprint = fingerprint_file(&audio).unwrap();
        let folder = dir.path().join("sidecar");
        let sidecar = Sidecar::open(&folder).unwrap();
        let source = Source::new(SourceKind::Tool, "fixture");
        let run = sidecar.begin_run(&source, "Fixture measurement").unwrap();
        let original = sidecar
            .record(&NewFact {
                track_id: "removed",
                fingerprint: Some(&fingerprint),
                kind: Kind::measurement("intensity"),
                value: json!({"summary": 0.7}),
                source: &source,
                confidence: 1.0,
                run,
            })
            .unwrap();
        if stale {
            common::write_audio(&audio, 100, 2, None);
            sidecar
                .mark_stale("removed", &fingerprint_file(&audio).unwrap())
                .unwrap();
        }
        drop(sidecar);
        // Downgrade only this throwaway fixture, then exercise the real
        // migration path from each previously shipped schema version.
        let db = rusqlite::Connection::open(folder.join("sidecar.db")).unwrap();
        db.execute_batch(
            "DROP VIEW attached_fact;
            DROP TABLE fact_attachment; DROP TABLE attachment_change;
            DROP TABLE attachment_review; DROP TABLE track_identity;
            ",
        )
        .unwrap();
        if version < 3 {
            db.execute_batch("DROP TABLE decision_answer; DROP TABLE decision;")
                .unwrap();
        }
        if version < 2 {
            db.execute_batch("DROP TABLE mutant_change; DROP TABLE mutant;")
                .unwrap();
        }
        db.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            [version.to_string()],
        )
        .unwrap();
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
            assert!(sidecar.fact(original.id).unwrap().unwrap().is_stale());
        } else {
            assert_eq!(result.reattached[0].fact_ids, [original.id]);
            let mut expected = original;
            expected.track_id = "readded".into();
            assert_eq!(
                sidecar.facts_of_track("readded").unwrap(),
                [expected.clone()]
            );
            drop(sidecar);
            assert_eq!(
                Sidecar::open(&folder)
                    .unwrap()
                    .facts_of_track("readded")
                    .unwrap(),
                [expected]
            );
        }
    }
}
