//! How long measuring takes, per minute of audio, on audio generated here.
//!
//! `cargo run --release -p wreckordbox-intensity --example timing [MINUTES]`
//!
//! Writes a 16-bit stereo WAV of the loud, busy test pattern to a temporary
//! folder, measures it through the decoder as the service does, and prints
//! the seconds of work per minute of audio. Never give it a real track: it
//! takes no path.
#![allow(clippy::pedantic, clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::print_stdout)]

#[path = "../tests/common/mod.rs"]
mod signals;

use std::io::Write;
use std::time::Instant;

fn main() {
    let minutes: f64 = std::env::args().nth(1).and_then(|text| text.parse().ok()).unwrap_or(6.0);
    let samples = signals::loud_busy(minutes * 60.0);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("generated.wav");
    let data_len = u32::try_from(samples.len() * 4).unwrap();
    let mut out = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    out.write_all(b"RIFF").unwrap();
    out.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    out.write_all(b"WAVEfmt ").unwrap();
    out.write_all(&16_u32.to_le_bytes()).unwrap();
    out.write_all(&1_u16.to_le_bytes()).unwrap();
    out.write_all(&2_u16.to_le_bytes()).unwrap();
    out.write_all(&signals::RATE.to_le_bytes()).unwrap();
    out.write_all(&(signals::RATE * 4).to_le_bytes()).unwrap();
    out.write_all(&4_u16.to_le_bytes()).unwrap();
    out.write_all(&16_u16.to_le_bytes()).unwrap();
    out.write_all(b"data").unwrap();
    out.write_all(&data_len.to_le_bytes()).unwrap();
    for sample in &samples {
        let value = ((sample.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes();
        out.write_all(&value).unwrap();
        out.write_all(&value).unwrap();
    }
    drop(out);
    drop(samples);

    let started = Instant::now();
    let intensity = wreckordbox_intensity::measure_file(&path, &[], &|| false).unwrap().unwrap();
    let took = started.elapsed().as_secs_f64();
    println!(
        "{minutes} minutes of generated 44.1 kHz stereo WAV: {took:.3} s, {:.3} s per minute of audio ({} points, Intensity {})",
        took / minutes,
        intensity.curve.intensity.len(),
        intensity.summary.intensity
    );
}
