# Wreckordbox

A personal, Linux-first fork of rbxport for preparing DJ music on an Omarchy laptop and carrying it to Pioneer/AlphaTheta players. This glossary fixes the words the fork uses for the things a DJ works with.

## Language

**Library**:
The one collection of tracks, playlists, cues and beat grids that is edited on this laptop, stored in rekordbox's own format. There is exactly one writer: wreckordbox.
_Avoid_: Collection, database, master db

**Library Protection**:
The DJ's switch, on until turned off, that refuses every change to the Library, whether it comes from the window, a script or the command line. Reading is never refused, and writing Facts to the Sidecar is not a change to the Library.
_Avoid_: Read-only mode, write lock

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

**Safe to remove**:
The state of a Stick after wreckordbox has ejected it successfully and seen it unmounted. A finished load alone is not safe to remove.
_Avoid_: Done, finished

**Rekordbox-readable**:
The property of a Stick that rekordbox on any machine can open it and show its tracks, playlists and cues, without wreckordbox present.

**Energy**:
The DJ's own 1 to 5 judgement of how intense a track is, stored as the track's star rating. It is not a quality score.
_Avoid_: Rating, stars (when the meaning is intended)

**Track Filter**:
The filter on the player itself, which narrows a list by MyTag, Energy, Prep colour, tempo and key while the DJ is playing. It reads only native rekordbox fields on the Stick.
_Avoid_: Smart filter, deck filter

**Tag schema**:
The DJ's written vocabulary for describing tracks: a genre tree in which a child implies its parent, any number of other dimensions, and which of them appear as the four MyTag categories. A plain-text file the DJ versions and refines.
_Avoid_: Taxonomy, tag config, MyTag layout

**Intensity**:
A measured, continuous Fact about how hard a track hits, including how it rises and falls over the track's length. Calibrated against Energy, which remains the DJ's own judgement.
_Avoid_: Energy level, energy curve, loudness

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

**Judgement**:
A Fact about the music itself, such as genre, mood, Situation or Energy. It follows the track when its audio file is replaced, moved or re-added.
_Avoid_: Subjective fact, tag fact

**Measurement**:
A Fact computed from a specific audio file's samples, such as Intensity or Stems. It is stale once that file's audio changes.
_Avoid_: Analysis result, objective fact

**Decision**:
A recorded judgement between fixed candidates: the question, the context given, the candidates, the choice, how sure, and which Decider answered. A Decision can be asked again of a different Decider and compared.
_Avoid_: Classification, prediction, inference

**Decider**:
Whatever answers a Decision: a rule, an agent, a decision model or the DJ.
_Avoid_: Model, classifier, judge

**Projection**:
A deliberate step that writes chosen Facts into fields rekordbox and players understand, such as a MyTag, Energy, comment, Prep colour or cue. Each kind of change can be set to wait for the DJ or to apply above a confidence.
_Avoid_: Sync, write-back

**Barrier**:
A write whose outcome is uncertain (interrupted, unconfirmed or unreadable), which makes Projection treat every value it could have touched as the DJ's until the DJ clears it.
_Avoid_: Lock, block

**Run**:
One piece of work by an agent, a tool or the DJ that changes the Library or the Sidecar, recorded as a unit so it can be reviewed and unwound together.
_Avoid_: Batch, job, session

**Ledger**:
The durable record of every change to the Library and the Sidecar: what changed, from what to what, by which source, in which Run.
_Avoid_: History, audit log, undo stack

**Assertion**:
The DJ's explicit confirmation of a value, recorded in the Ledger even when the value does not change, so that Projection treats it as the DJ's.
_Avoid_: Touch, re-save, same-value edit

**Unwind**:
Reversing a Run, or part of one, from the Ledger at any later time and out of order. A value changed since is flagged, not overwritten. Distinct from undo, which reverses the most recent edits in the window.
_Avoid_: Rollback, revert, restore

**Large-run backup**:
A backup of the Library and the Sidecar together, taken before a Run that will change more than a set number of tracks, and named in that Run's Ledger record.
_Avoid_: Snapshot, checkpoint

**Stem**:
A separated component of a parent track (vocals, melody, bass or drums), held in the Sidecar, attached to its parent and inheriting the parent's beat grid, key and cues.
_Avoid_: Split, part, layer

**Native stems**:
A track's Stems written to a Stick in the form a modded XDJ-AZ plays on its pads. Other players ignore them, so the Stick stays Rekordbox-readable.
_Avoid_: AZ stems, stem files

**Stem track**:
A Stem exported to a Stick as an ordinary track, for players that cannot play Native stems.
_Avoid_: Acapella track, detached stem

**Mutant**:
A playlist's own edited version of a track: its parent track plus the edits made for that playlist, such as Stems muted, key shifted, tempo changed or cues moved. It is held in the Sidecar, inherits the parent's grid, Energy and tags, and reaches a Stick as an ordinary track rendered at load time.
_Avoid_: Variant, edit, version, copy

**Intake**:
Preparing a newly added track for playing: correcting its title and artist, tagging it, checking its beat grid and cues, and marking it Clean. A track is waiting for Intake while it has no Prep colour.
_Avoid_: Import (import means adding audio files to the Library), prep queue, inbox

**Recipe**:
The DJ's written criteria for a journey through a set: how Intensity should move, how far tempo and key may shift, which Situations belong where, how much to favour recent additions. A plain-text file the DJ versions and refines.
_Avoid_: Rules, template, preset

**Set plan**:
An ordered choice of tracks built from a Recipe, held in the Sidecar with the reason for each transition and a suggested place to mix. It reaches the Library and a Stick as an ordinary playlist.
_Avoid_: Setlist, set, generated playlist

**Round-trip**:
Bringing cues, ratings and play history changed on a Stick at a gig back into the Library.
_Avoid_: Sync back, import (import means adding audio files to the Library)
