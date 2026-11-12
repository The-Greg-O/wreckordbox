//! Plays a generated track through the real device and counts what the device
//! lost: the measurement wbx-45.16 asks for (underruns over two minutes at
//! normal tempo, with a key shift, with a tempo change).
//!
//! Silent by default: the master is at zero, so the whole engine runs —
//! decoding, resampling, the stretcher, the strips, the limiter — and the
//! device is fed silence. Nothing is heard and nothing is written but a
//! generated WAV in the system's temporary folder.
//!
//! `cargo run --release -p rbl-deck --example playprobe -- <normal|key|tempo> [seconds] [gain]`
#![allow(
    clippy::pedantic,
    clippy::print_stdout,
    clippy::unwrap_used,
    clippy::expect_used
)]

use std::io::Write as _;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rbl_deck::{Deck, DeckEvent, Engine, EventSink};

const RATE: u32 = 44_100;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("normal", String::as_str).to_owned();
    let seconds: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(120);
    let gain: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);

    let dir = std::env::temp_dir().join(format!("playprobe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("probe.wav");
    write_track(&wav, seconds + 10);

    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let events: EventSink = Arc::new(move |event| {
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(event);
        }
    });
    let engine = Engine::new(&events).expect("an output device");
    engine.master().set_gain(gain);
    engine.load(Deck::A, &wav);
    if let DeckEvent::Error { message, .. } = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("loaded in time")
    {
        println!("could not load the generated track: {message}");
        return;
    }
    match mode.as_str() {
        "key" => engine.set_key_shift(Deck::A, 2),
        "tempo" => {
            engine.set_master_tempo(Deck::A, true);
            engine.set_tempo(Deck::A, 1.06);
        }
        _ => {}
    }
    println!(
        "mode {mode}, {seconds} s, device rate {} Hz, load {}",
        engine.sample_rate(),
        loadavg()
    );
    if mode == "toggle" {
        // Play and pause by turns, for a capture of what the device plays
        // across each start and stop.
        std::thread::sleep(Duration::from_millis(500));
        for _ in 0..seconds {
            engine.play(Deck::A);
            std::thread::sleep(Duration::from_millis(1500));
            engine.pause(Deck::A);
            std::thread::sleep(Duration::from_millis(1500));
        }
        println!(
            "RESULT mode=toggle cycles={seconds} underruns={} load_end={}",
            engine.audio_health().underruns,
            loadavg()
        );
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let before = engine.audio_health();
    let started = Instant::now();
    engine.play(Deck::A);
    let mut last = before.underruns;
    while started.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_secs(10));
        let now = engine.audio_health();
        println!(
            "  {:>4} s: underruns {:>4} (+{}), late callbacks {}, callback load {:.2}, output latency {:.1} ms, load {}",
            started.elapsed().as_secs(),
            now.underruns - before.underruns,
            now.underruns - last,
            now.xruns - before.xruns,
            now.load,
            now.latency.as_secs_f64() * 1000.0,
            loadavg()
        );
        last = now.underruns;
        std::io::stdout().flush().ok();
    }
    engine.pause(Deck::A);
    std::thread::sleep(Duration::from_millis(300));
    let after = engine.audio_health();
    println!(
        "RESULT mode={mode} seconds={seconds} underruns={} late_callbacks={} load_end={}",
        after.underruns - before.underruns,
        after.xruns - before.xruns,
        loadavg()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// A chord with a kick on every beat at 124 BPM: content at every height, so
/// the stretcher has real work.
fn write_track(path: &std::path::Path, seconds: u64) {
    let frames = seconds as usize * RATE as usize;
    let mut data = Vec::with_capacity(frames * 4);
    let beat = (RATE as f64 * 60.0 / 124.0) as usize;
    for i in 0..frames {
        let t = i as f64 / f64::from(RATE);
        let chord = [220.0, 277.18, 329.63, 440.0]
            .iter()
            .map(|f| (2.0 * std::f64::consts::PI * f * t).sin())
            .sum::<f64>()
            * 0.12;
        let since = (i % beat) as f64 / f64::from(RATE);
        let kick = (2.0 * std::f64::consts::PI * 55.0 * since).sin() * (-since * 18.0).exp() * 0.5;
        let s = ((chord + kick).clamp(-1.0, 1.0) * 32_767.0) as i16;
        data.extend_from_slice(&s.to_le_bytes());
        data.extend_from_slice(&s.to_le_bytes());
    }
    let mut f = std::fs::File::create(path).unwrap();
    let len = data.len() as u32;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16_u32.to_le_bytes()).unwrap();
    f.write_all(&1_u16.to_le_bytes()).unwrap();
    f.write_all(&2_u16.to_le_bytes()).unwrap();
    f.write_all(&RATE.to_le_bytes()).unwrap();
    f.write_all(&(RATE * 4).to_le_bytes()).unwrap();
    f.write_all(&4_u16.to_le_bytes()).unwrap();
    f.write_all(&16_u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&len.to_le_bytes()).unwrap();
    f.write_all(&data).unwrap();
}
