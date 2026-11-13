//! The Sidecar as a general store of Facts: what a new kind of Fact, and a
//! command line for Facts, would build on.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::common;

use serde_json::json;
use wreckordbox_sidecar::{
    fingerprint_file, Fingerprint, Kind, Nature, NewFact, Sidecar, SidecarError, Source, SourceKind,
};

const GENRE: Kind = Kind::judgement("genre");
const INTENSITY: Kind = Kind::measurement("intensity");

struct Store {
    dir: tempfile::TempDir,
    sidecar: Sidecar,
}

fn store() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(&dir.path().join("sidecar")).unwrap();
    Store { dir, sidecar }
}

impl Store {
    /// The fingerprint of some audio; `value` tells one audio from another.
    fn audio(&self, value: i16) -> Fingerprint {
        let path = self.dir.path().join(format!("audio-{value}.wav"));
        common::write_audio(&path, 100, value, None);
        fingerprint_file(&path).unwrap()
    }
}

fn agent() -> Source {
    Source::new(SourceKind::Agent, "tagger")
}

fn dj() -> Source {
    Source::new(SourceKind::Dj, "dj")
}

#[test]
fn a_fact_of_any_kind_is_kept_with_its_track_fingerprint_source_confidence_run_and_time() {
    let store = store();
    let audio = store.audio(1);
    let run = store
        .sidecar
        .begin_run(&agent(), "Tag the new arrivals")
        .unwrap();

    let recorded = store
        .sidecar
        .record(&NewFact {
            track_id: "10003",
            fingerprint: Some(&audio),
            kind: GENRE,
            value: json!({ "genre": "Techno / Dub" }),
            source: &agent(),
            confidence: 0.8,
            run,
        })
        .unwrap();
    store.sidecar.finish_run(run, "done").unwrap();

    assert_eq!(recorded.track_id, "10003");
    assert_eq!(recorded.fingerprint, Some(audio));
    assert_eq!(recorded.kind, "genre");
    assert_eq!(recorded.nature, Nature::Judgement);
    assert_eq!(recorded.value, json!({ "genre": "Techno / Dub" }));
    assert_eq!(recorded.source, agent());
    assert_eq!(recorded.confidence, 0.8);
    assert_eq!(recorded.run, run);
    assert!(recorded.recorded_at > 1_700_000_000_000);
    assert!(recorded.is_live() && !recorded.is_stale());
    assert_eq!(
        store.sidecar.facts("10003", "genre").unwrap(),
        std::slice::from_ref(&recorded)
    );
    assert_eq!(store.sidecar.fact(recorded.id).unwrap(), Some(recorded));
    assert_eq!(store.sidecar.facts("10004", "genre").unwrap(), []);
}

#[test]
fn facts_outlive_the_process_that_recorded_them() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("sidecar");
    let recorded = {
        let sidecar = Sidecar::open(&folder).unwrap();
        let run = sidecar.begin_run(&dj(), "By hand").unwrap();
        sidecar
            .record(&NewFact {
                track_id: "10003",
                fingerprint: None,
                kind: GENRE,
                value: json!("House"),
                source: &dj(),
                confidence: 1.0,
                run,
            })
            .unwrap()
    };

    let reopened = Sidecar::open(&folder).unwrap();

    assert_eq!(reopened.facts_of_track("10003").unwrap(), [recorded]);
}

#[test]
fn a_new_fact_supersedes_the_same_sources_earlier_one_and_leaves_other_sources_alone() {
    let store = store();
    let run = store.sidecar.begin_run(&agent(), "Tag").unwrap();
    let say = |source: &Source, genre: &str| {
        store
            .sidecar
            .record(&NewFact {
                track_id: "10003",
                fingerprint: None,
                kind: GENRE,
                value: json!(genre),
                source,
                confidence: 0.5,
                run,
            })
            .unwrap()
    };
    let first = say(&agent(), "House");
    let by_hand = say(&dj(), "Techno");

    let second = say(&agent(), "Tech House");

    let facts = store.sidecar.facts("10003", "genre").unwrap();
    let live: Vec<i64> = facts.iter().filter(|f| f.is_live()).map(|f| f.id).collect();
    assert_eq!(live, [second.id, by_hand.id]);
    // The earlier answer is kept beside the one that replaced it.
    let replaced = facts.iter().find(|f| f.id == first.id).unwrap();
    assert_eq!(replaced.superseded_by, Some(second.id));
    assert_eq!(replaced.value, json!("House"));
}

#[test]
fn a_change_of_audio_makes_measurements_stale_and_leaves_judgements_alone() {
    let store = store();
    let (old_audio, new_audio) = (store.audio(1), store.audio(2));
    let run = store.sidecar.begin_run(&agent(), "Analyse").unwrap();
    let record = |kind: Kind, track: &str| {
        store
            .sidecar
            .record(&NewFact {
                track_id: track,
                fingerprint: Some(&old_audio),
                kind,
                value: json!(3),
                source: &agent(),
                confidence: 1.0,
                run,
            })
            .unwrap()
    };
    let judgement = record(GENRE, "10003");
    let measurement = record(INTENSITY, "10003");
    let another_track = record(INTENSITY, "10004");

    // Nothing is stale while the audio is what it was.
    assert_eq!(
        store.sidecar.mark_stale("10003", &old_audio).unwrap(),
        Vec::<i64>::new()
    );

    let stale = store.sidecar.mark_stale("10003", &new_audio).unwrap();

    assert_eq!(stale, [measurement.id]);
    let read = |id| store.sidecar.fact(id).unwrap().unwrap();
    assert!(read(measurement.id).is_stale());
    assert!(read(measurement.id).is_live());
    assert!(!read(judgement.id).is_stale());
    assert!(!read(another_track.id).is_stale());
    // Said once: a second look marks nothing more.
    assert_eq!(
        store.sidecar.mark_stale("10003", &new_audio).unwrap(),
        Vec::<i64>::new()
    );
}

#[test]
fn a_fact_that_makes_no_sense_is_refused() {
    let store = store();
    let audio = store.audio(1);
    let run = store.sidecar.begin_run(&agent(), "Analyse").unwrap();
    let fact = |kind: Kind<'static>, fingerprint: Option<&Fingerprint>, confidence: f64| {
        let source = agent();
        store.sidecar.record(&NewFact {
            track_id: "10003",
            fingerprint,
            kind,
            value: json!(1),
            source: &source,
            confidence,
            run,
        })
    };

    // A Measurement is of some audio.
    assert!(matches!(
        fact(INTENSITY, None, 1.0),
        Err(SidecarError::Invalid(_))
    ));
    assert!(matches!(
        fact(GENRE, None, 1.5),
        Err(SidecarError::Invalid(_))
    ));
    // A kind is declared one way, once.
    fact(INTENSITY, Some(&audio), 1.0).unwrap();
    assert!(matches!(
        fact(Kind::judgement("intensity"), Some(&audio), 1.0),
        Err(SidecarError::KindConflict { .. })
    ));
    assert_eq!(store.sidecar.facts_of_track("10003").unwrap().len(), 1);
}

#[test]
fn what_is_known_of_some_audio_is_found_on_every_track_that_has_it() {
    let store = store();
    let (one, other) = (store.audio(1), store.audio(2));
    let run = store.sidecar.begin_run(&agent(), "Measure").unwrap();
    let record = |track: &str, audio: &Fingerprint, value: i64| {
        store
            .sidecar
            .record(&NewFact {
                track_id: track,
                fingerprint: Some(audio),
                kind: INTENSITY,
                value: json!(value),
                source: &agent(),
                confidence: 1.0,
                run,
            })
            .unwrap()
            .id
    };
    let first = record("10001", &one, 1);
    record("10002", &other, 2);
    let twin = record("10003", &one, 3);
    // The first track's audio changes: its Fact is stale, but still of `one`.
    store.sidecar.mark_stale("10001", &other).unwrap();

    let found: Vec<i64> = store
        .sidecar
        .facts_of_audio(&one, INTENSITY.name)
        .unwrap()
        .iter()
        .map(|fact| fact.id)
        .collect();
    assert_eq!(found, [twin, first]);
    assert!(store
        .sidecar
        .facts_of_audio(&one, "genre")
        .unwrap()
        .is_empty());
}
