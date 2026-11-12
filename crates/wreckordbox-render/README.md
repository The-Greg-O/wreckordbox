# wreckordbox-render

The offline renderer behind Mutants (ADR 0008): decoded audio (a track, or the
kept Stems of one), a key shift and a tempo in; an audio file out. It has no Tauri, service or Library dependency.
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
  render. `MemorySource` is samples in memory. `StemSum` is the kept Stems of
  a set (`Stems`: the parent's length and a file per Stem), added sample for
  sample. `MutantSource::open(parent, kept, stems)` is what a Mutant renders
  from: the parent itself when every Stem is kept (no Stem edit; the Stems
  are not opened), the `StemSum` otherwise (`StemsMissing` without Stems,
  `NothingKept` when none is kept).
- `Edits` is a key shift in semitones (pitch moves, tempo does not) and a tempo
  ratio (1.06 is six percent fast and a file that much shorter; pitch does not
  move). The limits here are the stretcher's (an octave, half to double); a
  Mutant's narrower limits are the caller's rule.
- `Destination` is a path and a `Format`: `Wav`, `Aiff` or `Mp3`, always
  16-bit stereo at the source's rate. `Format::for_parent(path)` is the
  parent's own format where there is an encoder for it and WAV otherwise (so
  FLAC and M4A parents render to WAV); `Destination::for_parent(parent,
  name)` adds the matching extension to `name`. MP3 is the export's LAME
  settings: 320 kbit/s constant, best quality, with an Info frame.
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
  frames it declares, or an MP3 that ends a frame or more before the count in
  its Xing, Info or VBRI frame (`Source(AudioError::Truncated)`), writes
  nothing. An M4A cut short does not open.
- **Stems fit their parent or are refused.** Every kept Stem must be exactly
  `Stems::frames` long (`StemLength`, from its header before anything is
  written, or from what it decodes to) and all at one rate (`StemRate`).
- **A Stem sum is exact.** Kept Stems are added as 16-bit integers: one kept
  Stem unedited renders as itself, several as their exact sum, and a muted
  one is not in the file at all. The sum is not scaled; past full scale it
  clips, as any render does. Stem-derived audio is at the Stems' rate.
- **An MP3 lines up on rekordbox's timeline.** The frames the new file puts
  ahead of its audio there (its Info frame, LAME's and the decoder's delay:
  2,257 at 44.1 and 48 kHz) are dropped from the front of the render, so an
  event at `t` in the source is at `t / tempo` in the MP3 as a deck and
  `rbl-audio` read it (within 1 ms), MP3 to MP3 included. The file decodes
  up to two MPEG frames longer than `Rendered::frames` (LAME's padding at the
  end).
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
  every format is written at 16 bits, whatever the source's depth. Stems are
  16-bit already.
- **An MP3 with no counting tag cut short is not detected.** Its length is an
  estimate from its size, so one that was truncated renders as the shorter
  file it now is. An MP3 whose tag's count is stale (cut by hand without
  rewriting the tag) is refused although it plays. A damaged packet inside
  any file is its own length of silence, as in rekordbox and on a deck.
- **The first 2,257 frames (51 ms) of an MP3 render are dropped** to line it
  up. For an MP3 parent that is its own lead and encoder delay; for a WAV or
  Stem source rendered to MP3 it is real audio, which only happens if a
  caller asks for MP3 from a non-MP3 parent.
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
files, read back through `rbl-audio` (`tests/render.rs`; Stems in
`tests/stems.rs`, formats in `tests/formats.rs`), and checks what a
render does to files and to broken sources (`tests/safety.rs`, through the
`render_on` seam). `cargo run --release -p wreckordbox-render
--example rendercost` times four minutes of generated audio, in wall time and
in processor time.
