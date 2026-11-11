# Wreckordbox

A personal, Linux-first fork of rbxport for preparing DJ music on an Omarchy laptop and carrying it to Pioneer/AlphaTheta players. This glossary fixes the words the fork uses for the things a DJ works with.

## Language

**Library**:
The one collection of tracks, playlists, cues and beat grids that is edited on this laptop, stored in rekordbox's own format. There is exactly one writer: wreckordbox.
_Avoid_: Collection, database, master db

**Fallback library**:
The rekordbox library left on macOS after the one-way migration, frozen and opened only in an emergency.
_Avoid_: Backup, old library

**Stick**:
A USB drive holding an export of part of the Library that a player can read.
_Avoid_: Device, drive, USB, export

**Gig stick**:
A Stick the user performs from, which must never be used for testing.

**Spare stick**:
A Stick the user has declared safe to overwrite.
_Avoid_: Test stick, disposable stick

**Rekordbox-readable**:
The property of a Stick that rekordbox on any machine can open it and show its tracks, playlists and cues, without wreckordbox present.

**Energy**:
The DJ's own 1 to 5 judgement of how intense a track is, stored as the track's star rating. It is not a quality score.
_Avoid_: Rating, stars (when the meaning is intended)

**MyTag category**:
One of the four groups rekordbox allows MyTags to be organised into. A category holds any number of tags; the number of categories is fixed by rekordbox.
_Avoid_: Tag group, tag folder

**Prep colour**:
The colour label on a track, used to record where it is in preparation (for example Clean, Problem, Delete, Sample, Buy), not to describe the music.
_Avoid_: Colour tag, track colour

**Sidecar**:
Wreckordbox's own store beside the Library, keyed by track, holding results from tools rekordbox does not have. It is never part of the rekordbox-format Library.
_Avoid_: Extra tables, metadata db, cache

**Fact**:
One thing known about a track, held in the Sidecar with its source (the DJ, an agent, a tool or a web lookup) and a confidence. A value the DJ sets by hand is a Fact whose source is the DJ, and it outranks any other source.
_Avoid_: Finding, attribute, metadata

**Projection**:
A deliberate step that writes chosen Facts into fields rekordbox and players understand, such as a MyTag, Energy, comment, Prep colour or cue. Each kind of change can be set to wait for the DJ or to apply above a confidence.
_Avoid_: Sync, write-back

**Run**:
One piece of work by an agent, a tool or the DJ that changes the Library or the Sidecar, recorded as a unit so it can be reviewed and unwound together.
_Avoid_: Batch, job, session

**Ledger**:
The durable record of every change to the Library and the Sidecar: what changed, from what to what, by which source, in which Run.
_Avoid_: History, audit log, undo stack

**Unwind**:
Reversing a Run, or part of one, from the Ledger at any later time and out of order. A value changed since is flagged, not overwritten. Distinct from undo, which reverses the most recent edits in the window.
_Avoid_: Rollback, revert, restore

**Stem**:
A separated component of a parent track (vocals, melody, bass or drums), held in the Sidecar, attached to its parent and inheriting the parent's beat grid, key and cues.
_Avoid_: Split, part, layer

**Native stems**:
A track's Stems written to a Stick in the form a modded XDJ-AZ plays on its pads. Other players ignore them, so the Stick stays Rekordbox-readable.
_Avoid_: AZ stems, stem files

**Stem track**:
A Stem exported to a Stick as an ordinary track, for players that cannot play Native stems.
_Avoid_: Acapella track, detached stem

**Round-trip**:
Bringing cues, ratings and play history changed on a Stick at a gig back into the Library.
_Avoid_: Sync back, import (import means adding audio files to the Library)
