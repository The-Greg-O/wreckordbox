//! The formats a render is written in: the parent's own where there is an
//! encoder for it (WAV, AIFF, MP3), WAV otherwise.
//!
//! Every file is read back through `rbl-audio` on rekordbox's timeline,
//! which is where a deck and the parent's grid put things.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

mod support;

use std::path::{Path, PathBuf};

use support::{channel, level_at, read, sine, write_wav_rate, RATE};
use wreckordbox_render::{render, Destination, Edits, Format, Proceed, Rendered, TrackSource};

/// One MPEG-1 Layer III frame.
const MP3_FRAME: usize = 1_152;

fn render_file(source: &Path, edits: Edits, out: &Path, format: Format) -> Rendered {
    let mut track = TrackSource::open(source).unwrap();
    render(
        &mut track,
        edits,
        &Destination::new(out, format),
        &mut |_| Proceed::Continue,
    )
    .unwrap()
}

#[test]
fn the_format_is_the_parents_where_there_is_an_encoder_and_wav_otherwise() {
    for (name, format) in [
        ("a.mp3", Format::Mp3),
        ("a.MP3", Format::Mp3),
        ("a.wav", Format::Wav),
        ("a.aiff", Format::Aiff),
        ("a.aif", Format::Aiff),
        ("a.AIF", Format::Aiff),
        ("a.flac", Format::Wav),
        ("a.m4a", Format::Wav),
        ("a.ogg", Format::Wav),
        ("a", Format::Wav),
    ] {
        assert_eq!(Format::for_parent(Path::new(name)), format, "{name}");
    }
    assert_eq!(Format::Wav.extension(), "wav");
    assert_eq!(Format::Aiff.extension(), "aiff");
    assert_eq!(Format::Mp3.extension(), "mp3");
    assert_eq!(
        Destination::for_parent(Path::new("/music/Song.Mix.aif"), Path::new("/work/m1")),
        Destination::new("/work/m1.aiff", Format::Aiff)
    );
}

struct Bench {
    dir: tempfile::TempDir,
}

impl Bench {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }
}

#[test]
fn an_aiff_render_holds_the_source_sample_for_sample_at_its_rate() {
    let bench = Bench::new();
    for rate in [44_100, 48_000] {
        let source = bench.path(&format!("source-{rate}.wav"));
        let left = sine(440.0, 10_000, 0.5);
        let right = sine(1_000.0, 10_000, 0.25);
        write_wav_rate(&source, &left, &right, rate);
        let out = bench.path(&format!("out-{rate}.aiff"));
        let rendered = render_file(&source, Edits::default(), &out, Format::Aiff);

        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[..4], b"FORM");
        assert_eq!(&bytes[8..12], b"AIFF");
        let (frames, got_rate) = read(&out);
        assert_eq!(got_rate, rate);
        assert_eq!(rendered.frames, 10_000);
        assert_eq!(frames, read(&source).0, "the samples, at {rate} Hz");
    }
}

/// Silence with a burst of tone from `at` for `secs`, as a WAV at `rate`.
fn burst_source_at(path: &Path, rate: u32, frames: usize, at: usize, secs: f64) -> PathBuf {
    let mut samples = vec![0_i16; frames];
    let tone = sine(1_000.0 * f64::from(RATE) / f64::from(rate), (secs * f64::from(rate)) as usize, 0.5);
    samples[at..at + tone.len()].copy_from_slice(&tone);
    write_wav_rate(path, &samples, &samples, rate);
    path.to_path_buf()
}

fn burst_source(path: &Path, frames: usize, at: usize, secs: f64) -> PathBuf {
    burst_source_at(path, RATE, frames, at, secs)
}

/// The first frame whose level is over a quarter of the burst's.
fn onset(frames: &[[i16; 2]]) -> usize {
    frames
        .iter()
        .position(|frame| frame[0].unsigned_abs() > 4_096)
        .expect("a burst")
}

#[test]
fn an_mp3_render_is_320_kbps_and_lines_up_with_its_source() {
    let bench = Bench::new();
    for rate in [44_100, 48_000] {
        let frames = rate as usize;
        let at = frames / 2;
        let source = burst_source_at(&bench.path(&format!("source-{rate}.wav")), rate, frames, at, 0.2);
        let out = bench.path(&format!("out-{rate}.mp3"));
        let rendered = render_file(&source, Edits::default(), &out, Format::Mp3);

        let bytes = std::fs::read(&out).unwrap();
        // MPEG-1 Layer III, 320 kbit/s (bitrate index 14), as the export writes.
        assert_eq!(&bytes[..2], &[0xFF, 0xFB]);
        assert_eq!(bytes[2] >> 4, 14);

        let (decoded, got_rate) = read(&out);
        assert_eq!(got_rate, rate);
        assert_eq!(rendered.frames, frames as u64);
        // On rekordbox's timeline the audio is where the source had it, so
        // the parent's grid and cues fit; an MP3 rounds up to whole frames
        // at the end.
        let moved = onset(&decoded) as i64 - onset(&read(&source).0) as i64;
        assert!(moved.abs() <= 44, "the burst moved {moved} frames at {rate} Hz");
        assert!(
            (frames..=frames + 2 * MP3_FRAME).contains(&decoded.len()),
            "{} frames decoded for {frames} at {rate} Hz",
            decoded.len()
        );
        // An MP3 parent rendered to MP3 again stays in place too: its own
        // lead is what the new file's makes room for.
        let again = bench.path(&format!("again-{rate}.mp3"));
        render_file(&out, Edits::default(), &again, Format::Mp3);
        let moved = onset(&read(&again).0) as i64 - onset(&decoded) as i64;
        assert!(moved.abs() <= 44, "MP3 to MP3 moved the burst {moved} frames");
        if rate == RATE {
            let level = level_at(&channel(&decoded[at + 2_000..at + 6_000], 0), 1_000.0);
            assert!((level - 0.5).abs() < 0.02, "the tone at {level}");
        }
    }
}

#[cfg(feature = "rubberband")]
#[test]
fn the_rendered_length_is_the_parents_scaled_by_the_tempo_in_every_format() {
    let bench = Bench::new();
    let frames = RATE as usize;
    let source = burst_source(&bench.path("source.wav"), frames, 22_050, 0.2);
    let edits = Edits {
        key_shift: 0,
        tempo: 1.05,
    };
    let want = (frames as f64 / 1.05).round() as usize;
    for format in [Format::Wav, Format::Aiff, Format::Mp3] {
        let out = bench.path(&format!("out.{}", format.extension()));
        let rendered = render_file(&source, edits, &out, format);
        assert_eq!(rendered.frames, want as u64, "{format:?}");
        let (decoded, _) = read(&out);
        if format == Format::Mp3 {
            assert!(
                (want..=want + 2 * MP3_FRAME).contains(&decoded.len()),
                "{} MP3 frames for {want}",
                decoded.len()
            );
            let moved = onset(&decoded) as i64 - (22_050.0 / 1.05) as i64;
            assert!(moved.abs() <= 220, "the burst moved {moved} frames");
        } else {
            assert_eq!(decoded.len(), want, "{format:?}");
        }
    }
}

#[test]
fn an_mp3_parent_cut_short_is_refused_not_rendered_shorter() {
    let bench = Bench::new();
    let source = burst_source(&bench.path("source.wav"), RATE as usize, 22_050, 0.2);
    let parent = bench.path("parent.mp3");
    render_file(&source, Edits::default(), &parent, Format::Mp3);
    let bytes = std::fs::read(&parent).unwrap();
    std::fs::write(&parent, &bytes[..bytes.len() / 2]).unwrap();

    let out = bench.path("out.mp3");
    let mut track = TrackSource::open(&parent).unwrap();
    let error = render(&mut track, Edits::default(), &Destination::mp3(&out), &mut |_| {
        Proceed::Continue
    })
    .unwrap_err();
    assert!(
        matches!(
            error,
            wreckordbox_render::RenderError::Source(rbl_audio::AudioError::Truncated { .. })
        ),
        "{error:?}"
    );
    assert!(!out.exists());
}
