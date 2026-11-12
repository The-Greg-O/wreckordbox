//! Readiness is the same source and stretcher rule used by a real render.
#![allow(clippy::unwrap_used, clippy::pedantic)]
mod support;
use support::{StemBench, RATE};
use wreckordbox_render::{preflight, Edits, Kept, RenderError};

#[test]
fn preflight_checks_actual_container_and_length_without_writing() {
    let bench = StemBench::new(RATE as usize);
    let parent = bench.parent();
    let stems = bench.stems();
    let kept = Kept {
        vocals: true,
        other: false,
        bass: false,
        drums: false,
    };
    preflight(&parent, kept, Some(&stems), Edits::default()).unwrap();
    let disguised = bench.path("parent.mp3");
    std::fs::copy(&parent, &disguised).unwrap();
    preflight(&disguised, kept, Some(&stems), Edits::default()).unwrap();
    let lossy = bench.path("parent.mp4");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../rbl-audio/tests/fixtures/tone-faststart.m4a"
        ),
        &lossy,
    )
    .unwrap();
    assert!(matches!(
        preflight(&lossy, kept, Some(&stems), Edits::default()),
        Err(RenderError::StemAlignmentUnknown { .. })
    ));
    let mut wrong = stems;
    wrong.frames += 1;
    assert!(matches!(
        preflight(&parent, kept, Some(&wrong), Edits::default()),
        Err(RenderError::StemLength { .. })
    ));
}

#[test]
fn preflight_reports_this_builds_actual_stretcher_capabilities() {
    let bench = StemBench::new(RATE as usize);
    for edits in [
        Edits {
            key_shift: 1,
            tempo: 1.0,
        },
        Edits {
            key_shift: 0,
            tempo: 1.1,
        },
    ] {
        let result = preflight(&bench.parent(), Kept::ALL, None, edits);
        #[cfg(feature = "rubberband")]
        assert!(result.is_ok());
        #[cfg(not(feature = "rubberband"))]
        assert!(matches!(
            result,
            Err(RenderError::KeyShiftUnavailable | RenderError::TempoUnavailable)
        ));
    }
}
