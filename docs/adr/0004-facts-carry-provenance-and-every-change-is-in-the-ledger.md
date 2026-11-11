# Facts carry their source, and every change is recorded in a Ledger that can be unwound

Agents and the DJ both change the same Library, and an agent with the CLI can do anything the DJ can, so safety cannot come from permissions. Instead, every Fact records its source and a confidence, a value the DJ sets by hand is a Fact that outranks any other source, and every change to the Library or the Sidecar is written to a durable Ledger, grouped by Run, with its before and after values. Any Run, or part of one, can be unwound later and out of order; a value changed since is flagged rather than overwritten. A backup is taken before a large Run as a second net.

## Considered Options

- **Current values only, with upstream's undo.** Rejected: that undo is one stack per session, capped and not saved, and several kinds of write bypass it, so a bad bulk pass found days later could only be fixed by hand or by restoring a backup and losing everything since.
- **A snapshot before each Run as the only remedy.** Rejected as the sole mechanism: restoring discards all later work. Kept as the second net.
- **Facts keyed only by the Library's track ID, or only by an audio fingerprint.** Rejected: the first loses everything on a re-add and keeps stale audio results after a file swap; the second loses the DJ's judgements when a file is replaced. Facts attach to the track with the fingerprint alongside, and are either a Judgement (follows the music) or a Measurement (tied to the exact audio, stale when it changes).

## Consequences

- The Ledger has to cover every kind of write, including analysis, imports and Round-trip, or it has holes.
- A hand edit in the window must be recorded to the Sidecar as well as the Library, so the two cannot drift.
- The DJ's corrections are kept beside the answer they replaced, which is what any Decider is evaluated against.
