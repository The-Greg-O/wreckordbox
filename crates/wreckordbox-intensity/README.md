# wreckordbox-intensity

[Crate index](../README.md) · [Sidecar](../wreckordbox-sidecar/README.md) · [Service](../wreckordbox-service/README.md)

Intensity: a Measurement of how hard a track hits and how that changes over its length. A summary, a curve with one point a second and, for a track with a beat grid, a curve with one point a bar.

It stands beside the tempo, key and waveform analysis (`rbl-analysis`) and changes none of it. It reads audio through the same decoder (`rbl_audio::MonoStream`), so a time here is a time on the beat grid's timeline. It knows nothing of the Library, the Sidecar or the service: audio in, a value out. The Library service records the value as a Fact of the built-in kind `intensity` (`crates/wreckordbox-service/src/intensity.rs`).

## Start here

[`src/lib.rs`](src/lib.rs) holds the value's shape, the weights and scales, and `combine`, the one place the features become Intensity. [`src/loudness.rs`](src/loudness.rs) is the K-weighting filter and gated loudness. [`src/analyser.rs`](src/analyser.rs) is the streaming pass and the curves.

## The method (version 2)

Five features, each computed for every point of a curve from the audio inside that point:

| feature | what it is | unit |
|---|---|---|
| `loudness` | K-weighted loudness after ITU-R BS.1770 / EBU R128, the two channels summed, ungated over the point | LUFS |
| `low_end` | mean power of the mono mix from 20 to 150 Hz | dB re full scale |
| `high_end` | mean power of the mono mix from 5 to 20 kHz | dB re full scale |
| `onset_strength` | spectral flux of log-compressed magnitudes from 20 Hz to 11 kHz (Hann frames of 46.4 ms at any rate, half overlapped), summed per second | flux per second |
| `onset_density` | flux peaks that stand clear of the flux around them, per second | onsets per second |

Each is mapped onto 0 to 1 by a fixed straight line (`SCALES`), multiplied by a fixed weight (`WEIGHTS`, which add up to 1) and added. Both tables are in `src/lib.rs` and nowhere else, and both are stored in every value (`method.weights`, `method.scales`) beside the raw feature curves, so the combination can be redone with other weights without reading the audio again.

The weights and scales were chosen by reasoning. Nothing is fitted to data and nothing is learned. Fitting Intensity to the DJ's Energy is calibration, a separate step.

Every curve is made from **cells**, the audio every 0.2 s (its loudness, band levels, summed flux and onsets), which are stored in the value (`cells`). A point or bar takes each cell by how much of it it covers. So the bars for another beat grid come from the stored value without the audio (`Intensity::with_bars`); `grid` names the grid the stored bars were made for (`grid_of`, a hash of the bar starts).

The summary is the mean of the time curve, with the mean of its highest and lowest quarters, the integrated loudness (gated as BS.1770 says) and the loudest three seconds.

## Contracts

- **Deterministic.** The same audio gives the same value, byte for byte as JSON, however the audio is cut into pieces on the way in. No randomness, no clock, no threads inside a measurement; stored numbers are rounded.
- **Any sample rate.** The frame is the same length in seconds at every rate and the flux is averaged over the same band of hertz, so the same band-limited audio measures the same at 44.1, 48, 88.2 and 96 kHz (`tests/robustness.rs`).
- **Mono and stereo only.** A file of more channels is refused (`MeasureError::Channels`). A mono file is heard from both speakers: it measures as the same audio in both channels of a stereo file; audio in one channel only is 3 dB quieter.
- **The whole file.** The last part of a second, a last loudness block shorter than 100 ms and the samples after the last whole spectral frame all count; a 10 ms clip measures.
- **Valid or nothing.** A sample that is not a number fails the measurement (`Invalid::NonFinite`); `Intensity::check` is what a value must pass to be recorded or reused.
- **Cost.** Roughly 0.45 s per minute of 44.1 kHz stereo WAV on one core of an Apple M2 under load, release build, decoding included (the `timing` example). Compressed formats add their decoding.
- **Bounded memory.** One packet of audio is held at a time. What is kept per track is one number per 100 ms, three per spectral frame and five per cell: about 750 kB for ten minutes.
- **`METHOD_VERSION`** changes whenever the same audio would measure differently. The service measures again when a stored value is of another version.
- **Bars** come from the bar starts the caller passes (the service reads them from the track's analysis file). No grid: `bars` is `null` and the time curve is unchanged.
- **Synthetic audio only in tests.** `tests/common/mod.rs` generates quiet, sparse and loud, busy signals whose answer is known by construction. No test reads a real track.

## Run focused checks

```sh
cargo test -p wreckordbox-intensity
cargo run --release -p wreckordbox-intensity --example timing     # seconds of work per minute of generated audio
```
