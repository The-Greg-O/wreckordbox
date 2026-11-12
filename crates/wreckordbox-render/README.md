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
  `.<name>.<pid>-<n>-<clock>.part` beside the destination and moved into place
  last. A cancel or any error removes it.
- **A render removes only what it made.** The temporary file is created
  exclusively; a name already taken is left as it is and another is tried.
- **No existing file is replaced.** A destination that exists is refused
  (`DestinationExists`), so a source cannot be overwritten by naming it, and a
  file that arrives there during the render is kept. The finished file is
  moved by a rename that refuses to replace (`renameat2(RENAME_NOREPLACE)`,
  `renameatx_np(RENAME_EXCL)`), or by a hard link where the filesystem has no
  such rename; both fail in one step if the name is taken. Where a filesystem
  has neither, the render fails (`NoReplaceUnsupported`): it never checks and
  then renames. A source is only ever opened for reading.
- **A broken source is an error, not a shorter render.** A read that fails, a
  decoder that gives up, or a WAV, AIFF or FLAC file that ends before the
  frames it declares (`Source(AudioError::Truncated)`) writes nothing.
- **No edits, no change.** With a zero key shift and a tempo of exactly 1.0 the
  stretcher is not in the path, as on a deck, and the render matches the source
  sample for sample: the same frame count and, for a 16-bit source, the same
  sample values exactly.
- **An edited render starts and ends where it should.** The output is exactly
  `round(source frames / tempo)` frames. The stretcher's latency is removed, so
  an event at time `t` in the source is at `t / tempo` in the render, measured
  within 2 ms and gated at 5 ms.
- **Pitch.** A key shift lands within 3 cents, the gate of the player's
  stretcher tests.

## Limits

- **The outermost 20 ms of an edited render are quieter than the source.** The
  stretcher's window sees silence outside the file, so audio that is at full
  level on the very first or last frame comes up to it over about 20 ms
  (measured at 0.68 of the level over the first 20 ms for a tempo change, 0.42
  with a key shift three semitones down). Nothing is lost or moved.
- **16-bit.** `TrackSource` hands out the 16-bit frames `rbl-audio` decodes and
  `Format::Wav` writes 16-bit, whatever the source's depth.
- **A lossy file cut short is not detected.** An MP3 or AAC file's length is
  an estimate, so one that was truncated renders as the shorter file it now
  is. A damaged packet inside any file is its own length of silence, as in
  rekordbox and on a deck.
- **The rate is the source's.** A deck stretches at the device's rate after
  resampling; a render stretches at the file's. Same engine, same settings.

## The stretcher

`rbl_deck::Offline` is the deck's key-lock stretcher (Rubber Band R3 in its
real-time mode) driven over a whole file. It is the same engine in the same
mode as playback, on purpose: Rubber Band's own offline mode is a different
sound. See `rbl-deck/src/offline.rs` for how the start and end are handled.

A build without Rubber Band (`--no-default-features`, see `LICENSING.md`)
renders unedited audio only. A key shift is refused (`KeyShiftUnavailable`)
and so is a tempo change (`TempoUnavailable`): the WSOLA fallback a deck plays
through cannot hold a pitch (a 440 Hz tone at 0.94x came back 107 cents flat),
and a file in the wrong key is worse than no file.

## Tests and cost

`cargo test -p wreckordbox-render` renders generated tones and measures the
files, read back through `rbl-audio` (`tests/render.rs`), and checks what a
render does to files and to broken sources (`tests/safety.rs`, through the
`render_on` seam). `cargo run --release -p wreckordbox-render
--example rendercost` times four minutes of generated audio, in wall time and
in processor time.
