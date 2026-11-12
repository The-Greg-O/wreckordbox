//! Generated audio, hand-written files and measurements shared by the Stems
//! and format tests. Nothing here uses the crate under test: a source is
//! written by hand and a render is read back through `rbl-audio`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use wreckordbox_render::Stems;

pub const RATE: u32 = 44_100;

/// One channel of a sine, as 16-bit samples: integers from the start, so a
/// sum of them is exact and a test can say what a render must hold to the
/// sample.
pub fn sine(hz: f64, frames: usize, amplitude: f64) -> Vec<i16> {
    (0..frames)
        .map(|i| {
            (amplitude * 32_767.0 * (i as f64 / f64::from(RATE) * hz * std::f64::consts::TAU).sin())
                .round() as i16
        })
        .collect()
}

/// A 16-bit PCM WAV written by hand: `left` and `right` are the channels.
pub fn write_wav(path: &Path, left: &[i16], right: &[i16]) {
    let frames = left.len().min(right.len());
    write_wav_rate(path, &left[..frames], &right[..frames], RATE);
}

pub fn write_wav_rate(path: &Path, left: &[i16], right: &[i16], rate: u32) {
    let data = (left.len() * 4) as u32;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for (l, r) in left.iter().zip(right) {
        bytes.extend_from_slice(&l.to_le_bytes());
        bytes.extend_from_slice(&r.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

/// A file's frames as 16-bit stereo and its rate, read through `rbl-audio`
/// on rekordbox's timeline (an MP3's lead included), failing on a file that
/// cannot be read to its end.
pub fn read(path: &Path) -> (Vec<[i16; 2]>, u32) {
    let mut stream = rbl_audio::MonoStream::open(path).unwrap();
    let mut frames = Vec::new();
    while let Some((_, stereo)) = stream.try_next_frames().unwrap() {
        frames.extend_from_slice(stereo);
    }
    (frames, stream.sample_rate())
}

pub fn channel(frames: &[[i16; 2]], which: usize) -> Vec<f64> {
    frames
        .iter()
        .map(|frame| f64::from(frame[which]) / 32_768.0)
        .collect()
}

/// The level of one frequency in a signal, as a sine's amplitude: a
/// Goertzel filter over the whole of it. A tone of amplitude `a` at `hz`
/// measures about `a`; a tone that is not there measures near nothing.
pub fn level_at(signal: &[f64], hz: f64) -> f64 {
    let w = std::f64::consts::TAU * hz / f64::from(RATE);
    let coefficient = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0, 0.0);
    for &x in signal {
        let s = x + coefficient * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    let power = s1 * s1 + s2 * s2 - coefficient * s1 * s2;
    2.0 * power.sqrt() / signal.len() as f64
}

/// Four Stems in a temporary folder, each a tone of its own on the left and
/// another on the right, so what a render kept, and on which side, can be
/// told by frequency.
pub struct StemBench {
    pub dir: tempfile::TempDir,
    pub frames: usize,
}

/// The Stems' names, in the order vocals, other, bass, drums.
pub const NAMES: [&str; 4] = ["vocals", "other", "bass", "drums"];

/// Each Stem's left tone, in the order vocals, other, bass, drums. Chosen to
/// sit on whole Goertzel bins of a one-second signal and far from each
/// other and from the right-hand tones.
pub const HZ: [f64; 4] = [880.0, 660.0, 110.0, 2_200.0];
pub const AMPLITUDE: f64 = 0.2;
/// Each Stem's right tone.
pub const HZ_RIGHT: [f64; 4] = [1_100.0, 440.0, 220.0, 3_300.0];
pub const AMPLITUDE_RIGHT: f64 = 0.1;

/// A Stem's two channels.
pub type Channels = (Vec<i16>, Vec<i16>);

impl StemBench {
    pub fn new(frames: usize) -> Self {
        Self::with(
            frames,
            std::array::from_fn(|i| {
                (
                    sine(HZ[i], frames, AMPLITUDE),
                    sine(HZ_RIGHT[i], frames, AMPLITUDE_RIGHT),
                )
            }),
        )
    }

    /// Four Stems of the given samples, vocals, other, bass, drums.
    pub fn with(frames: usize, stems: [Channels; 4]) -> Self {
        let bench = Self {
            dir: tempfile::tempdir().unwrap(),
            frames,
        };
        for (name, (left, right)) in NAMES.into_iter().zip(&stems) {
            write_wav(&bench.path(&format!("{name}.wav")), left, right);
        }
        bench
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// The set as the Sidecar would hand it over: the parent's length and a
    /// file per Stem.
    pub fn stems(&self) -> Stems {
        Stems {
            frames: self.frames as u64,
            vocals: self.path("vocals.wav"),
            other: self.path("other.wav"),
            bass: self.path("bass.wav"),
            drums: self.path("drums.wav"),
        }
    }

    /// The Stems' own samples, read through `rbl-audio`, in the same order.
    pub fn samples(&self) -> [Vec<[i16; 2]>; 4] {
        NAMES.map(|name| read(&self.path(&format!("{name}.wav"))).0)
    }

    /// What keeping `kept` (vocals, other, bass, drums) must give: the kept
    /// Stems' samples added as integers, clipped at full scale.
    pub fn sum(&self, kept: [bool; 4]) -> Vec<[i16; 2]> {
        let samples = self.samples();
        (0..self.frames)
            .map(|i| {
                let mut sum = [0_i32; 2];
                for (stem, _) in samples.iter().zip(kept).filter(|(_, k)| *k) {
                    sum[0] += i32::from(stem[i][0]);
                    sum[1] += i32::from(stem[i][1]);
                }
                sum.map(|s| s.clamp(-32_768, 32_767) as i16)
            })
            .collect()
    }

    /// A parent track: the four Stems summed, as a WAV at 44.1 kHz.
    pub fn parent(&self) -> PathBuf {
        let path = self.path("parent.wav");
        let mix = self.sum([true; 4]);
        let left: Vec<i16> = mix.iter().map(|f| f[0]).collect();
        let right: Vec<i16> = mix.iter().map(|f| f[1]).collect();
        write_wav(&path, &left, &right);
        path
    }

    /// A parent of `frames` frames at `rate`, a quiet tone: a real file of
    /// its own length, to hold a Stem set up against.
    pub fn parent_of(&self, name: &str, frames: usize, rate: u32) -> PathBuf {
        let path = self.path(name);
        let tone = sine(330.0, frames, 0.3);
        write_wav_rate(&path, &tone, &tone, rate);
        path
    }
}
