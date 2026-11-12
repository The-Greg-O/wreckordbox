# wreckordbox-render

The offline renderer behind Mutants (ADR 0008): decoded audio, a key shift and
a tempo in; an audio file out. It has no Tauri, service or Library dependency.
The Stick load and the audition command both call it, so a Mutant sounds the
same in both.

## Interface

```rust
let mut track = TrackSource::open(&parent_audio)?;
let rendered = render(
    &mut track,
    Edits { key_shift: -2, tempo: 1.04 },
    &Destination::wav(&destination),
    &mut |progress| Proceed::Continue,
)?;
```

- `Source` is where audio comes from: interleaved stereo `f32` a block at a
  time, a sample rate, and a length hint. `TrackSource` is a whole file decoded
  through `rbl-audio`, on rekordbox's timeline, so the parent's grid fits the
  render. `MemorySource` is samples in memory. A sum of kept Stems is another
  `Source`; nothing else changes.
- `Edits` is a key shift in semitones (pitch moves, tempo does not) and a tempo
  ratio (1.06 is six percent fast and a file that much shorter; pitch does not
  move). The limits here are the stretcher's (an octave, half to double); a
  Mutant's narrower limits are the caller's rule.
- `Destination` is a path and a `Format`. `Format::Wav` is 16-bit PCM stereo at
  the source's rate. Other formats are new `Format` variants behind the
  crate's `Encoder` trait.
- The callback hears `Progress` (source frames done, and the total when known)
  about twice per second of source audio, and may answer `Proceed::Cancel`.

## Guarantees

- **The destination is complete or absent.** Audio is written to a hidden
  `.<name>.<pid>-<n>.part` beside the destination and renamed into place last.
  A cancel or any error removes it.
- **No existing file is replaced.** A destination that exists is refused
  (`DestinationExists`), so a source cannot be overwritten by naming it. A
  source is only ever opened for reading.
- **No edits, no change.** With a zero key shift and a tempo of exactly 1.0 the
  stretcher is not in the path, as on a deck, and the render matches the source
  sample for sample: the same frame count and, for a 16-bit source, the same
  sample values exactly.
- **An edited render starts and ends where it should.** The output is exactly
  `round(source frames / tempo)` frames. The stretcher's latency is removed, so
  an event at time `t` in the source is at `t / tempo` in the render, measured
  within 2 ms and gated at 5 ms.
- **Pitch.** A key shift lands within 3 cents, the gate of the player's
  stretcher tests; measured well under a tenth of a cent on a steady tone.

## The stretcher

`rbl_deck::Offline` is the deck's key-lock stretcher (Rubber Band R3 in its
real-time mode) driven over a whole file. It is the same engine in the same
mode as playback, on purpose: Rubber Band's own offline mode is a different
sound. See `rbl-deck/src/offline.rs` for how the start and end are handled.

A build without Rubber Band (`--no-default-features`, see `LICENSING.md`)
renders tempo through the WSOLA fallback and refuses a key shift
(`KeyShiftUnavailable`).

## Tests and cost

`cargo test -p wreckordbox-render` renders generated tones and measures the
files, read back through `rbl-audio`. `cargo run --release -p wreckordbox-render
--example rendercost` times four minutes of generated audio.
