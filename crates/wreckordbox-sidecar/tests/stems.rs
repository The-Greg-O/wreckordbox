//! Stems in the Sidecar: four files attached to their parent, held to its
//! length.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod common;

use std::path::PathBuf;

use serde_json::json;
use wreckordbox_sidecar::stems::{NewStems, Role, STEMS};
use wreckordbox_sidecar::{
    fingerprint_file, Fingerprint, Nature, RunId, Sidecar, SidecarError, Source, SourceKind,
};

struct Made {
    dir: tempfile::TempDir,
    sidecar: Sidecar,
    parent: Fingerprint,
    source: Source,
}

fn made() -> Made {
    let dir = tempfile::tempdir().unwrap();
    let sidecar = Sidecar::open(&dir.path().join("sidecar")).unwrap();
    let parent_path = dir.path().join("parent.wav");
    common::write_audio(&parent_path, 500, 3, None);
    let parent = fingerprint_file(&parent_path).unwrap();
    Made {
        dir,
        sidecar,
        parent,
        source: Source::new(SourceKind::Tool, "separation-engine"),
    }
}

impl Made {
    /// A Run and four Stem files in its workspace; `drums` frames long for
    /// the drums, 500 for the rest.
    fn separation(&self, drums: u32) -> (RunId, Vec<(Role, PathBuf)>) {
        let run = self.sidecar.begin_run(&self.source, "Separate").unwrap();
        let workspace = self.sidecar.workspace(run).unwrap();
        let files = [Role::Drums, Role::Vocals, Role::Bass, Role::Other]
            .into_iter()
            .map(|role| {
                let path = workspace.join(format!("parent_({role})_model.wav"));
                let frames = if role == Role::Drums { drums } else { 500 };
                common::write_audio(&path, frames, 0, None);
                (role, path)
            })
            .collect();
        (run, files)
    }

    fn record(
        &self,
        run: RunId,
        files: &[(Role, PathBuf)],
    ) -> Result<wreckordbox_sidecar::stems::StemSet, SidecarError> {
        self.sidecar.record_stems(&NewStems {
            track_id: "10003",
            fingerprint: &self.parent,
            parent_samples: 500,
            sample_rate: 44_100,
            files,
            source: &self.source,
            produced_by: json!({ "model": "a-model" }),
            run,
        })
    }
}

#[test]
fn four_stems_are_moved_into_the_sidecar_and_recorded_as_one_measurement_of_their_parent() {
    let made = made();
    let (run, files) = made.separation(500);

    let set = made.record(run, &files).unwrap();

    assert_eq!(set.track_id, "10003");
    assert_eq!(set.fingerprint, made.parent);
    assert_eq!((set.samples, set.sample_rate), (500, 44_100));
    let roles: Vec<Role> = set.stems.iter().map(|stem| stem.role).collect();
    assert_eq!(roles, Role::ALL);
    for stem in &set.stems {
        assert!(stem.path.starts_with(made.sidecar.dir()));
        assert!(stem.path.is_file());
        assert_eq!(stem.format, "wav");
    }
    // Moved, not copied: the workspace no longer has them.
    assert!(files.iter().all(|(_, path)| !path.exists()));
    assert_eq!(set.produced_by, json!({ "model": "a-model" }));
    assert!(set.is_current());
    assert_eq!(
        made.sidecar.stems("10003").unwrap(),
        std::slice::from_ref(&set)
    );
    assert_eq!(
        made.sidecar.current_stems("10003", &made.parent).unwrap(),
        Some(set.clone())
    );

    // They are a Fact like any other: a Measurement of kind `stems`.
    let fact = made.sidecar.fact(set.fact_id).unwrap().unwrap();
    assert_eq!(fact.kind, STEMS.name);
    assert_eq!(fact.nature, Nature::Measurement);
    assert_eq!(fact.run, run);
    assert_eq!(fact.source, made.source);
}

#[test]
fn a_stem_of_another_length_than_its_parent_is_refused_and_nothing_is_recorded() {
    let made = made();
    let (run, files) = made.separation(499);

    let refused = made.record(run, &files);

    assert!(
        matches!(
            refused,
            Err(SidecarError::StemLength {
                role: Role::Drums,
                samples: 499,
                parent: 500
            })
        ),
        "{refused:?}"
    );
    assert_eq!(made.sidecar.stems("10003").unwrap(), []);
    assert!(!made.sidecar.dir().join("stems").exists());
    // The files are left where they were, for whoever made them to remove.
    assert!(files.iter().all(|(_, path)| path.is_file()));
}

#[test]
fn a_set_without_all_four_roles_is_refused() {
    let made = made();
    let (run, mut files) = made.separation(500);
    files.pop();

    assert!(matches!(
        made.record(run, &files),
        Err(SidecarError::StemSet(_))
    ));

    // Two files for one role and none for another is not a set either.
    let (run, mut files) = made.separation(500);
    files[0].0 = Role::Vocals;
    assert!(matches!(
        made.record(run, &files),
        Err(SidecarError::StemSet(_))
    ));
    assert_eq!(made.sidecar.stems("10003").unwrap(), []);
}

#[test]
fn stems_from_other_audio_are_not_the_tracks_current_stems() {
    let made = made();
    let (run, files) = made.separation(500);
    let old = made.record(run, &files).unwrap();
    let changed_path = made.dir.path().join("changed.wav");
    common::write_audio(&changed_path, 500, 9, None);
    let changed = fingerprint_file(&changed_path).unwrap();

    assert_eq!(made.sidecar.current_stems("10003", &changed).unwrap(), None);
    assert_eq!(
        made.sidecar.mark_stale("10003", &changed).unwrap(),
        [old.fact_id]
    );

    let sets = made.sidecar.stems("10003").unwrap();
    assert_eq!(sets.len(), 1);
    assert!(sets[0].stale && !sets[0].is_current());
    // Stale Stems keep their files.
    assert!(sets[0].stems.iter().all(|stem| stem.path.is_file()));
}

#[test]
fn opening_the_sidecar_clears_what_a_dead_run_left_in_its_workspace() {
    let made = made();
    let (_, files) = made.separation(500);
    let folder = made.sidecar.dir().to_path_buf();

    let reopened = Sidecar::open(&folder).unwrap();

    assert!(files.iter().all(|(_, path)| !path.exists()));
    assert_eq!(reopened.stems("10003").unwrap(), []);
}
