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

**Projection**:
A deliberate step that writes chosen Sidecar results into fields rekordbox and players understand, such as a MyTag, comment, colour or cue.
_Avoid_: Sync, write-back

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
