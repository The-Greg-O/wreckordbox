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
  sample, checked against the parent file (`StemSum::open(parent, stems,
  kept)`). `MutantSource::open(parent, kept, stems)` is what a Mutant renders
  from: the parent itself when every Stem is kept (no Stem edit; the Stems
  are not opened), the `StemSum` otherwise (`StemsMissing` without Stems,
  `NothingKept` when none is kept).
- `Edits` is a key shift in semitones (pitch moves, tempo does not) and a tempo
  ratio (1.06 is six percent fast and a file that much shorter; pitch does not
  move). The limits here are the stretcher's (an octave, half to double); a
  Mutant's narrower limits are the caller's rule.
- `Destination` is a path and a `Format`: `Wav`, `Aiff` or `Mp3`, always
  16-bit stereo at the source's rate. `Format::for_parent(path)` is the
  default and always lossless: AIFF for an AIFF parent, WAV for every other
  (MP3, FLAC and M4A parents included). `Destination::for_parent(parent,
  name)` adds the matching extension to `name`. MP3 only when a caller names
  `Format::Mp3`: the export's LAME settings, 320 kbit/s constant, best
  quality, with an Info frame, at 32, 44.1 or 48 kHz (another rate is
  refused rather than resampled).
- `Rendered` says what was written: `frames` (every frame the render made,
  all in the file), `sample_rate`, and, apart, `encoder_delay` and `padding`
  (0 and 0 for WAV and AIFF; for MP3 as the LAME tag records them).
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
  its Xing, Info or VBRI frame, or an M4A that ends a packet or more before
  its sample table (`Source(AudioError::Truncated)`), writes nothing. Most
  cut M4As do not open at all; one whose `mdat` runs to the end of the file
  does, and is refused here.
- **Stems fit their parent or are refused.** Every kept Stem must be exactly
  `Stems::frames` long (`StemLength`, from its header before anything is
  written, or from what it decodes to) and at 44.1 kHz (`StemRate`). The
  parent file itself must last as long: exactly `Stems::frames` at 44.1 kHz,
  within one 44.1 kHz frame at another rate (`StemsDoNotFitParent`). A lossy
  parent (MP3, M4A) is refused (`StemAlignmentUnknown`): its decoded length
  carries encoder delay and padding and where its Stems sit on its timeline
  is not established (wbx-49.1). Whether the Stems were made from this audio
  (and not from other audio of the same length) is not the renderer's to
  know: the service hands over only verified-current Stems
  (`Service::current_stems_for_render`).
- **A Stem sum is exact.** Kept Stems are added as 16-bit integers: one kept
  Stem unedited renders as itself, several as their exact sum, and a muted
  one is not in the file at all. The sum is not scaled; past full scale it
  clips, as any render does. Stem-derived audio is at the Stems' rate.
- **WAV and AIFF are exact to the frame.** They decode to exactly
  `Rendered::frames`, sample for sample what was rendered.
- **An MP3 keeps every frame and says what it added.** Nothing is dropped or
  moved to make up for encoder delay. The file holds LAME's delay (576
  frames), then every rendered frame, then padding to a whole MPEG frame;
  `Rendered` reports the three apart. `rbl-audio` decodes it to the Info
  frame (1,152 frames of silence in its model of rekordbox) plus delay plus
  frames plus padding, and the decoder adds its own 529-frame delay inside
  that. **Where a deck or rekordbox puts the first rendered frame, and so
  where the parent's beat grid lands on an MP3 Mutant, is not known**: it
  depends on whether the player honours the LAME tag's delay and how it
  treats the Info frame, which needs a deck to check. Use WAV or AIFF (the
  default) wherever the grid matters.
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
- **An MP3 render is lossy and not exact in length** (see above). It is only
  written when asked for by name.
- **A Stem edit of a lossy parent is refused** until wbx-49.1 measures where
  the separation engine's decode sits on the parent's timeline.
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
