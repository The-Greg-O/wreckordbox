//! Generated audio whose quiet and loud, sparse and busy stretches are
//! known by construction.
#![allow(dead_code)]

pub const RATE: u32 = 44_100;

fn frames(seconds: f64) -> usize {
    (seconds * f64::from(RATE)) as usize
}

fn sine(out: &mut [f32], hz: f64, amplitude: f32) {
    for (i, sample) in out.iter_mut().enumerate() {
        *sample += (i as f64 * 2.0 * std::f64::consts::PI * hz / f64::from(RATE)).sin() as f32 * amplitude;
    }
}

/// A burst at `at` seconds that dies away over `length` seconds: a sine at
/// `hz`, or noise when `hz` is zero.
fn hit(out: &mut [f32], at: f64, length: f64, hz: f64, amplitude: f32) {
    let start = frames(at);
    let n = frames(length);
    let mut noise: u32 = 0x1234_5678;
    for i in 0..n {
        let Some(sample) = out.get_mut(start + i) else { break };
        let decay = 1.0 - i as f32 / n as f32;
        let wave = if hz > 0.0 {
            (i as f64 * 2.0 * std::f64::consts::PI * hz / f64::from(RATE)).sin() as f32
        } else {
            noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (noise >> 8) as f32 / 8_388_608.0 - 1.0
        };
        *sample += wave * decay * decay * amplitude;
    }
}

/// A faint held note with a soft tick every two seconds.
pub fn quiet_sparse(seconds: f64) -> Vec<f32> {
    let mut out = vec![0.0; frames(seconds)];
    sine(&mut out, 220.0, 0.02);
    let mut at = 0.5;
    while at < seconds {
        hit(&mut out, at, 0.02, 1_000.0, 0.04);
        at += 2.0;
    }
    out
}

/// A kick on every beat at `bpm`, a noise hat on every sixteenth and a
/// held bass note, `gain` times as loud as the full-strength pattern.
pub fn loud_busy_at(seconds: f64, bpm: f64, gain: f32) -> Vec<f32> {
    let mut out = vec![0.0; frames(seconds)];
    sine(&mut out, 55.0, 0.2 * gain);
    let beat = 60.0 / bpm;
    let mut at = 0.0;
    let mut sixteenth = 0;
    while at < seconds {
        if sixteenth % 4 == 0 {
            hit(&mut out, at, 0.15, 60.0, 0.6 * gain);
        }
        hit(&mut out, at, 0.03, 0.0, 0.2 * gain);
        sixteenth += 1;
        at = f64::from(sixteenth) * beat / 4.0;
    }
    out
}

pub fn loud_busy(seconds: f64) -> Vec<f32> {
    loud_busy_at(seconds, 128.0, 1.0)
}

pub fn joined(parts: &[Vec<f32>]) -> Vec<f32> {
    parts.concat()
}
