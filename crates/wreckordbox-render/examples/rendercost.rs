//! How long a render takes against the length of what it renders.
//!
//! Four minutes of generated audio (a chord, a thump on every beat at 128 BPM
//! and noise under it, so the stretcher has transients and content at every
//! frequency to work on) written to a temporary WAV, then rendered from that
//! file with each kind of edit. No real music.
//!
//! `cargo run --release -p wreckordbox-render --example rendercost`
#![allow(
    clippy::pedantic,
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used
)]

use std::time::Instant;

use wreckordbox_render::{render, Destination, Edits, MemorySource, Proceed, TrackSource};

const RATE: u32 = 44_100;
const SECONDS: usize = 240;

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.wav");
    let mut generated = MemorySource::new(generate(), RATE);
    render(
        &mut generated,
        Edits::default(),
        &Destination::wav(&source),
        &mut |_| Proceed::Continue,
    )
    .unwrap();

    println!("{SECONDS} s of generated 44.1 kHz stereo, decoded from WAV and written to WAV");
    let cases = [
        ("no edits", Edits::default()),
        (
            "tempo +6%",
            Edits {
                key_shift: 0,
                tempo: 1.06,
            },
        ),
        (
            "key +2",
            Edits {
                key_shift: 2,
                tempo: 1.0,
            },
        ),
        (
            "key -3, tempo -8%",
            Edits {
                key_shift: -3,
                tempo: 0.92,
            },
        ),
    ];
    for (name, edits) in cases {
        let out = dir.path().join("out.wav");
        let _ = std::fs::remove_file(&out);
        let mut track = TrackSource::open(&source).unwrap();
        let started = Instant::now();
        let cpu_before = cpu_seconds();
        match render(&mut track, edits, &Destination::wav(&out), &mut |_| {
            Proceed::Continue
        }) {
            Ok(rendered) => {
                let took = started.elapsed().as_secs_f64();
                // Wall time is what a DJ waits; processor time is what the
                // render costs, and is the figure to trust on a busy machine.
                let cpu = match (cpu_before, cpu_seconds()) {
                    (Some(before), Some(after)) if after > before => format!(
                        ", {:.2} s of processor ({:.1}x)",
                        after - before,
                        SECONDS as f64 / (after - before)
                    ),
                    _ => String::new(),
                };
                println!(
                    "{name:>18}: {took:6.2} s, {:5.1}x real time ({:.1} s out){cpu}",
                    SECONDS as f64 / took,
                    rendered.frames as f64 / f64::from(rendered.sample_rate),
                );
            }
            Err(error) => println!("{name:>18}: {error}"),
        }
    }
}

/// Processor time this process has used, on Linux; nothing elsewhere.
fn cpu_seconds() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Past the command name, which is in brackets and may hold spaces.
    let rest = stat.rsplit_once(')')?.1;
    let mut fields = rest.split_whitespace().skip(11);
    let user: f64 = fields.next()?.parse().ok()?;
    let system: f64 = fields.next()?.parse().ok()?;
    // Clock ticks, a hundred a second on every Linux this runs on.
    Some((user + system) / 100.0)
}

fn generate() -> Vec<f32> {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    let beat = RATE as usize * 60 / 128;
    (0..SECONDS * RATE as usize)
        .flat_map(|i| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let noise = (state >> 40) as f32 / 8_388_608.0 - 1.0;
            let t = i as f32 / RATE as f32;
            let chord: f32 = [220.0_f32, 277.18, 329.63]
                .iter()
                .map(|hz| (t * hz * std::f32::consts::TAU).sin())
                .sum();
            let since = (i % beat) as f32 / RATE as f32;
            let thump = (since * 55.0 * std::f32::consts::TAU).sin() * (-since * 18.0).exp();
            let left = 0.15 * chord + 0.5 * thump + 0.05 * noise;
            [left, 0.15 * chord + 0.5 * thump - 0.05 * noise]
        })
        .collect()
}
