//! What a render must never do to files it does not own, and what it must
//! refuse to pass off as a finished render.
//!
//! The filesystem cases go through `render_on`, the seam that lets a test
//! stand in for a filesystem without hard links or without a rename that
//! refuses to replace, and put a competing file in the way at the exact
//! moment a render publishes.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use wreckordbox_render::publish::{Filesystem, Real};
use wreckordbox_render::{
    render, render_on, Destination, Edits, MemorySource, Proceed, RenderError, Source, TrackSource,
};

const RATE: u32 = 44_100;

fn tone(hz: f64, secs: f64) -> Vec<f32> {
    let frames = (secs * f64::from(RATE)) as usize;
    (0..frames)
        .flat_map(|i| {
            let value =
                (0.5 * (i as f64 / f64::from(RATE) * hz * std::f64::consts::TAU).sin()) as f32;
            [value, value]
        })
        .collect()
}

/// A 16-bit stereo WAV whose header declares `declared` frames and whose
/// data holds `samples`.
fn wav_bytes(samples: &[f32], declared_frames: usize) -> Vec<u8> {
    let data = (declared_frames * 4) as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for &sample in samples {
        bytes.extend_from_slice(&((sample * 32_767.0).round() as i16).to_le_bytes());
    }
    bytes
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn go(_: wreckordbox_render::Progress) -> Proceed {
    Proceed::Continue
}

/// A second of tone rendered unedited through `files` to `out`.
fn render_tone(files: &dyn Filesystem, hz: f64, out: &Path) -> Result<(), RenderError> {
    let mut source = MemorySource::new(tone(hz, 1.0), RATE);
    render_on(
        files,
        &mut source,
        Edits::default(),
        &Destination::wav(out),
        &mut go,
    )
    .map(|_| ())
}

// ---- damaged sources ----------------------------------------------------

#[test]
fn a_truncated_source_is_an_error_and_not_a_shorter_render() {
    // The header promises two seconds; the file was cut off after one. A
    // render of the first second would look like a finished Mutant.
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.wav");
    std::fs::write(&source, wav_bytes(&tone(440.0, 1.0), 2 * RATE as usize)).unwrap();
    let out = dir.path().join("out.wav");

    let mut track = TrackSource::open(&source).unwrap();
    let result = render(
        &mut track,
        Edits::default(),
        &Destination::wav(&out),
        &mut go,
    );

    assert!(
        matches!(result, Err(RenderError::Source(_))),
        "a truncated source gave {result:?}"
    );
    assert_eq!(files(dir.path()), ["source.wav"]);
}

#[test]
fn a_whole_source_still_renders() {
    // The control for the test above: the same file, complete.
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.wav");
    std::fs::write(&source, wav_bytes(&tone(440.0, 1.0), RATE as usize)).unwrap();
    let out = dir.path().join("out.wav");

    let mut track = TrackSource::open(&source).unwrap();
    let rendered = render(
        &mut track,
        Edits::default(),
        &Destination::wav(&out),
        &mut go,
    )
    .unwrap();

    assert_eq!(rendered.frames, u64::from(RATE));
}

/// A source that fails partway.
struct Failing {
    block: Vec<f32>,
    left: u32,
}

impl Source for Failing {
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn frames_hint(&self) -> Option<u64> {
        None
    }
    fn next_block(&mut self) -> Result<Option<&[f32]>, RenderError> {
        if self.left == 0 {
            return Err(RenderError::Io(io::Error::other("the disk went away")));
        }
        self.left -= 1;
        Ok(Some(&self.block))
    }
}

#[test]
fn a_source_that_fails_midway_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.wav");
    let mut source = Failing {
        block: tone(440.0, 0.1),
        left: 5,
    };

    let result = render(
        &mut source,
        Edits::default(),
        &Destination::wav(&out),
        &mut go,
    );

    assert!(matches!(result, Err(RenderError::Io(_))), "{result:?}");
    assert!(files(dir.path()).is_empty(), "{:?}", files(dir.path()));
}

// ---- the temporary file -------------------------------------------------

/// A filesystem where someone else's file is already at the temporary name
/// a render picks: the first `squats` names it tries.
struct Squatted {
    squats: Cell<u32>,
    taken: RefCell<Vec<PathBuf>>,
}

impl Squatted {
    fn new(squats: u32) -> Self {
        Self {
            squats: Cell::new(squats),
            taken: RefCell::new(Vec::new()),
        }
    }
}

impl Filesystem for Squatted {
    fn create_new(&self, path: &Path) -> io::Result<File> {
        if self.squats.get() > 0 {
            self.squats.set(self.squats.get() - 1);
            std::fs::write(path, b"someone else's").unwrap();
            self.taken.borrow_mut().push(path.to_path_buf());
        }
        Real.create_new(path)
    }
    fn rename_noreplace(&self, from: &Path, to: &Path) -> io::Result<()> {
        Real.rename_noreplace(from, to)
    }
    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
        Real.hard_link(from, to)
    }
}

#[test]
fn a_file_already_at_the_temporary_name_survives_and_the_render_goes_round_it() {
    // A crashed render's leftovers under a reused process id, or anything
    // else: the render did not make it, so it is not the render's to remove.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.wav");
    let squatted = Squatted::new(1);

    render_tone(&squatted, 440.0, &out).unwrap();

    let taken = squatted.taken.borrow();
    assert_eq!(taken.len(), 1);
    assert_eq!(std::fs::read(&taken[0]).unwrap(), b"someone else's");
    assert!(out.exists());
    assert_eq!(files(dir.path()).len(), 2, "{:?}", files(dir.path()));
}

#[test]
fn when_every_temporary_name_is_taken_the_render_fails_and_removes_none_of_them() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.wav");
    let squatted = Squatted::new(u32::MAX);

    let result = render_tone(&squatted, 440.0, &out);

    assert!(matches!(result, Err(RenderError::Io(_))), "{result:?}");
    let taken = squatted.taken.borrow();
    assert!(!taken.is_empty());
    for path in taken.iter() {
        assert_eq!(std::fs::read(path).unwrap(), b"someone else's", "{path:?}");
    }
    assert!(!out.exists());
    assert_eq!(files(dir.path()).len(), taken.len());
}

// ---- publishing ---------------------------------------------------------

/// A filesystem with switches: whether it can rename without replacing,
/// whether it can hard link, and something to run at the moment a render
/// asks to put its file in place, before the filesystem answers.
struct Staged<'a> {
    rename_noreplace: bool,
    hard_link: bool,
    at_publish: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
}

impl<'a> Staged<'a> {
    fn new(rename_noreplace: bool, hard_link: bool) -> Self {
        Self {
            rename_noreplace,
            hard_link,
            at_publish: RefCell::new(None),
        }
    }

    fn at_publish(self, run: impl FnOnce() + 'a) -> Self {
        *self.at_publish.borrow_mut() = Some(Box::new(run));
        self
    }

    fn interleave(&self) {
        if let Some(run) = self.at_publish.borrow_mut().take() {
            run();
        }
    }
}

impl Filesystem for Staged<'_> {
    fn create_new(&self, path: &Path) -> io::Result<File> {
        Real.create_new(path)
    }
    fn rename_noreplace(&self, from: &Path, to: &Path) -> io::Result<()> {
        if !self.rename_noreplace {
            // What Linux answers for a flag the filesystem does not know.
            return Err(io::Error::from_raw_os_error(22));
        }
        self.interleave();
        Real.rename_noreplace(from, to)
    }
    fn hard_link(&self, from: &Path, to: &Path) -> io::Result<()> {
        if !self.hard_link {
            // What Linux answers on FAT and exFAT.
            return Err(io::Error::from_raw_os_error(1));
        }
        self.interleave();
        Real.hard_link(from, to)
    }
}

#[test]
fn where_nothing_can_refuse_to_replace_the_render_fails_rather_than_rename() {
    // No exclusive rename and no hard links: the only move left is one that
    // replaces whatever is there, and that one is never made.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.wav");

    let result = render_tone(&Staged::new(false, false), 440.0, &out);

    assert!(
        matches!(result, Err(RenderError::NoReplaceUnsupported(_))),
        "{result:?}"
    );
    assert!(files(dir.path()).is_empty(), "{:?}", files(dir.path()));
}

/// Two renders to one destination, the second finishing at the moment the
/// first publishes: the one that got there first keeps its file, whole.
fn two_renders_one_destination(files_of_the_loser: impl FnOnce(&Path) -> Staged<'_>) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.wav");
    let reference = dir.path().join("reference.wav");
    render_tone(&Real, 880.0, &reference).unwrap();

    let staged = files_of_the_loser(&out);
    let result = render_tone(&staged, 440.0, &out);

    assert!(
        matches!(result, Err(RenderError::DestinationExists(_))),
        "{result:?}"
    );
    assert!(
        std::fs::read(&out).unwrap() == std::fs::read(&reference).unwrap(),
        "the winner's file changed"
    );
    assert_eq!(files(dir.path()), ["out.wav", "reference.wav"]);
}

#[test]
fn of_two_renders_to_one_destination_the_first_to_publish_wins() {
    two_renders_one_destination(|out| {
        Staged::new(true, true).at_publish(move || render_tone(&Real, 880.0, out).unwrap())
    });
}

#[test]
fn the_first_to_publish_wins_on_a_filesystem_that_can_only_link() {
    two_renders_one_destination(|out| {
        Staged::new(false, true).at_publish(move || render_tone(&Real, 880.0, out).unwrap())
    });
}

#[test]
fn a_filesystem_that_can_only_link_still_publishes_and_leaves_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.wav");

    render_tone(&Staged::new(false, true), 440.0, &out).unwrap();

    assert_eq!(files(dir.path()), ["out.wav"]);
}

#[test]
fn a_filesystem_that_cannot_link_still_publishes_by_exclusive_rename() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.wav");

    render_tone(&Staged::new(true, false), 440.0, &out).unwrap();

    assert_eq!(files(dir.path()), ["out.wav"]);
}
