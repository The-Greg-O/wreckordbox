# The Library stays in rekordbox's format, with a Sidecar for everything else

Wreckordbox edits the Library in rekordbox's own format rather than keeping its own canonical database, and puts results from tools rekordbox lacks (stems, extra audio analysis) in a Sidecar beside it. A separate Projection step writes chosen Sidecar results into fields players understand. We chose this because rbxport already reads and writes that format with far more hardware compatibility than a new store would have, migration from rekordbox on macOS becomes a folder copy, and the frozen Fallback library stays usable.

## Considered Options

- **An own-format canonical database** with rekordbox as an import source only. This was the plan in the earlier Dubplate project, which is now a parts donor for its separation engine and ideas. Rejected: it means rebuilding export correctness that upstream maintains.
- **Extra tables inside the rekordbox database.** Rejected: it risks the Fallback library and every schema change upstream makes.

## Consequences

- No rekordbox-format file gains wreckordbox tables or columns.
- The Library has no replayable change model, so multi-device sync is not designed for.
