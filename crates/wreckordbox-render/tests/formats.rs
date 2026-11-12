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

use support::{read, sine, write_wav_rate, RATE};
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
fn the_default_format_is_lossless_and_follows_the_parent_only_between_wav_and_aiff() {
    // A rendered Mutant is lossless unless MP3 is asked for by name: an MP3
    // parent renders to WAV by default.
    for (name, format) in [
        ("a.mp3", Format::Wav),
        ("a.MP3", Format::Wav),
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
        assert!(Format::for_parent(Path::new(name)).is_lossless(), "{name}");
    }
    assert!(!Format::Mp3.is_lossless());
    assert_eq!(Format::Wav.extension(), "wav");
    assert_eq!(Format::Aiff.extension(), "aiff");
    assert_eq!(Format::Mp3.extension(), "mp3");
    assert_eq!(
        Destination::for_parent(Path::new("/music/Song.Mix.aif"), Path::new("/work/m1")),
        Destination::new("/work/m1.aiff", Format::Aiff)
    );
    assert_eq!(
        Destination::for_parent(Path::new("/music/Song.mp3"), Path::new("/work/m2")),
        Destination::new("/work/m2.wav", Format::Wav)
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
    let tone = sine(
        1_000.0 * f64::from(RATE) / f64::from(rate),
        (secs * f64::from(rate)) as usize,
        0.5,
    );
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

/// The last frame whose level is over a quarter of the burst's.
fn offset(frames: &[[i16; 2]]) -> usize {
    frames
        .iter()
        .rposition(|frame| frame[0].unsigned_abs() > 4_096)
        .expect("a burst")
}

/// Frames `rbl-audio` (rekordbox's timeline as this repo models it) puts
/// ahead of an MP3's first coded frame: the LAME Info frame, read as a frame
/// of silence (`rbl_core::mpeg`).
const INFO_FRAME: usize = MP3_FRAME;

#[test]
fn an_mp3_render_keeps_every_frame_and_reports_delay_and_padding_apart() {
    let bench = Bench::new();
    for rate in [44_100_u32, 48_000] {
        // Full-level bursts at the very first and the very last frames: a
        // renderer that cut or moved anything at either end would lose one.
        let frames = rate as usize;
        let edge = 256;
        let mut samples = vec![0_i16; frames];
        let tone = sine(1_000.0 * f64::from(RATE) / f64::from(rate), edge, 0.9);
        samples[..edge].copy_from_slice(&tone);
        samples[frames - edge..].copy_from_slice(&tone);
        let source = bench.path(&format!("edges-{rate}.wav"));
        write_wav_rate(&source, &samples, &samples, rate);
        let out = bench.path(&format!("out-{rate}.mp3"));
        let rendered = render_file(&source, Edits::default(), &out, Format::Mp3);

        let bytes = std::fs::read(&out).unwrap();
        // MPEG-1 Layer III, 320 kbit/s (bitrate index 14), as the export writes.
        assert_eq!(&bytes[..2], &[0xFF, 0xFB]);
        assert_eq!(bytes[2] >> 4, 14);

        // The intended length, and the encoder's additions beside it.
        assert_eq!(rendered.frames, frames as u64, "every frame given");
        assert_eq!(rendered.encoder_delay, 576, "LAME's delay at {rate} Hz");
        assert!(rendered.padding > 0);
        let coded = rendered.encoder_delay + rendered.frames + rendered.padding;
        assert_eq!(coded % MP3_FRAME as u64, 0, "whole MPEG frames");

        let (decoded, got_rate) = read(&out);
        assert_eq!(got_rate, rate);
        assert_eq!(
            decoded.len() as u64,
            INFO_FRAME as u64 + coded,
            "decoded length is the Info frame, the delay, the frames and the padding"
        );
        // Both bursts are there, as far apart as in the source: nothing was
        // dropped at the front or the back and nothing in between moved.
        let (first, last) = (onset(&decoded), offset(&decoded));
        let span = last as i64 - first as i64;
        let want = (frames - 1) as i64;
        assert!(
            (span - want).abs() <= 16,
            "bursts {span} frames apart, not {want}, at {rate} Hz"
        );
        assert!(
            first >= INFO_FRAME + rendered.encoder_delay as usize,
            "the first burst decoded at {first}, ahead of the encoder's delay"
        );
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
            // Exact in what was encoded; what a player adds around it is
            // reported, never taken out of the audio.
            assert_eq!(
                decoded.len() as u64,
                INFO_FRAME as u64 + rendered.encoder_delay + rendered.frames + rendered.padding
            );
        } else {
            assert_eq!(decoded.len(), want, "{format:?} is exact to the frame");
            assert_eq!((rendered.encoder_delay, rendered.padding), (0, 0));
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
    let error = render(
        &mut track,
        Edits::default(),
        &Destination::mp3(&out),
        &mut |_| Proceed::Continue,
    )
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
