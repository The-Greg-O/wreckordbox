# A Mutant belongs to a playlist and is rendered when a Stick is loaded

The DJ wants a playlist to hold its own version of a track (vocals muted, key shifted, tempo changed, cues moved) without altering the track itself. A Mutant is that version: a parent track plus a list of edits, held in the Sidecar and owned by one playlist. It inherits the parent's grid, Energy and tags, may have its own cues, does not appear in All Tracks, and becomes audio only when a Stick is loaded, where it is written as an ordinary track. We chose this because it keeps the Library in rekordbox's format (ADR 0001), keeps edits cheap to change until the moment they are needed, and follows the pattern already set by Stem tracks.

## Considered Options

- **Duplicate the audio file in the Library when the edit is made.** Rejected: every tweak would render a file, the Library would fill with near-copies, and the copies would lose their tie to the parent's tags and Energy.
- **Store the edits on the playlist entry on the Stick and let the player apply them.** Not possible: checked against the export crates on 2026-11-11, a playlist entry is only a position, a track and a playlist in both Stick formats (three numbers in the older database, three columns in the newer one), with nowhere to carry a per-entry change.

## Consequences

- Loading a Stick can involve rendering audio, so it takes longer and needs the Stems to exist.
- A Mutant whose parent's audio changes has stale Stem-based edits, like any Measurement.
- On a Stick a Mutant is a separate track, so play history brought back by Round-trip has to be mapped to the Mutant and through it to its parent.
- In the Library a playlist containing a Mutant needs a native entry to stand in for it when rekordbox opens the Library; which entry is undecided.
