//! Loudness after ITU-R BS.1770 / EBU R128, on signals whose loudness the
//! standards state.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use wreckordbox_intensity::loudness::{integrated_lufs, lufs, KWeighting, BLOCK_SECONDS};

/// A sine in both channels, as 100 ms blocks of K-weighted mean square
/// (the two channels summed), which is what the analyser keeps.
fn blocks_of_sine(rate: u32, hz: f64, amplitude: f64, seconds: f64) -> Vec<f64> {
    let mut left = KWeighting::new(rate);
    let mut right = KWeighting::new(rate);
    let per_block = (f64::from(rate) * BLOCK_SECONDS) as usize;
    let total = (f64::from(rate) * seconds) as usize;
    let mut blocks = Vec::new();
    let mut sum = 0.0;
    for i in 0..total {
        let x = amplitude * (i as f64 * 2.0 * std::f64::consts::PI * hz / f64::from(rate)).sin();
        let (l, r) = (left.next(x), right.next(x));
        sum += l * l + r * r;
        if (i + 1) % per_block == 0 {
            blocks.push(sum / per_block as f64);
            sum = 0.0;
        }
    }
    blocks
}

#[test]
fn the_k_weighting_at_48_khz_is_the_filter_the_standard_tabulates() {
    let [shelf, high_pass] = KWeighting::coefficients(48_000);
    // ITU-R BS.1770-4, tables 1 and 2.
    let table_1 = [
        1.53512485958697,
        -2.69169618940638,
        1.19839281085285,
        -1.69065929318241,
        0.73248077421585,
    ];
    let table_2 = [1.0, -2.0, 1.0, -1.99004745483398, 0.99007225036621];
    for (got, want) in shelf.iter().zip(table_1) {
        assert!((got - want).abs() < 1e-9, "shelf {shelf:?}");
    }
    for (got, want) in high_pass.iter().zip(table_2) {
        assert!((got - want).abs() < 1e-9, "high pass {high_pass:?}");
    }
}

#[test]
fn a_full_scale_997_hz_sine_in_one_channel_reads_minus_3_lkfs_so_in_both_it_reads_zero() {
    // BS.1770: a 0 dBFS 997 Hz sine in the left channel alone is -3.01 LKFS;
    // the same in both channels is twice the power.
    for rate in [44_100, 48_000, 96_000] {
        let blocks = blocks_of_sine(rate, 997.0, 1.0, 5.0);
        let loudness = integrated_lufs(&blocks);
        assert!(loudness.abs() < 0.1, "{rate} Hz: {loudness}");
    }
}

#[test]
fn a_sine_23_db_down_reads_23_lu_lower() {
    // EBU Tech 3341, case 1: a 1 kHz stereo sine at -23 dBFS is -23 LUFS.
    let amplitude = 10_f64.powf(-23.0 / 20.0);
    let loudness = integrated_lufs(&blocks_of_sine(48_000, 1_000.0, amplitude, 20.0));
    assert!((loudness + 23.0).abs() < 0.1, "{loudness}");
}

#[test]
fn integrated_loudness_leaves_out_the_quiet_stretches_as_the_gate_says() {
    // EBU Tech 3341, case 3: -36 dBFS for 10 s, -23 for 60 s, -36 for 10 s
    // is -23 LUFS: the relative gate drops the quiet ends.
    let down = |db: f64| 10_f64.powf(db / 20.0);
    let mut blocks = blocks_of_sine(48_000, 1_000.0, down(-36.0), 10.0);
    blocks.extend(blocks_of_sine(48_000, 1_000.0, down(-23.0), 60.0));
    blocks.extend(blocks_of_sine(48_000, 1_000.0, down(-36.0), 10.0));
    let loudness = integrated_lufs(&blocks);
    assert!((loudness + 23.0).abs() < 0.1, "{loudness}");
}

#[test]
fn silence_has_no_loudness_and_is_reported_at_the_floor() {
    assert_eq!(
        integrated_lufs(&[0.0; 50]),
        wreckordbox_intensity::loudness::SILENCE_LUFS
    );
    assert_eq!(lufs(0.0), wreckordbox_intensity::loudness::SILENCE_LUFS);
}
