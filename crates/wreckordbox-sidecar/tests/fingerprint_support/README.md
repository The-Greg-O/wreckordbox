# Frozen fingerprint fixtures

`mod.rs` deterministically builds PCM and reuses the repository's existing
synthetic compressed fixtures. No new audio or database binary is committed.
`golden.json` fixes a1 and a2 for MP3 with Xing, CBR MP3 without Xing/Info,
FLAC, AIFF, WAV and M4A.

The a1 digests and `foundation.sql` were produced by **building and running
foundation ec1d477d**, the foundation commit before the fingerprint branch.
They were not computed by the new `legacy()` helper. The SQL contains actual
old-code Stems and decoded Intensity Facts; `store.rs` restores it and changes
only the fixture's track ownership mapping to the temporary Library IDs.
Stem WAVs are regenerated from their recorded frame counts and their hashes
are checked by the normal consumers. Original Fact fingerprints and values
are preserved. M4A's codec header omits channels in the old metadata helper;
its known synthetic duration supplies the fixture Stem length only, without
changing fingerprinting or Intensity measurement.

To reproduce, use a separate worktree at ec1d477d. Copy `mod.rs` to the same
relative test-support path and copy `foundation_oracle.rs` to
`crates/wreckordbox-service/examples/fingerprint_oracle.rs`. Run through the
machine-wide Rust queue:

```
cargo run -p wreckordbox-service --example fingerprint_oracle -- /tmp/fingerprint-oracle
```

The program prints and saves a1 goldens and creates a Sidecar. Export its SQLite
database with Python's `sqlite3.Connection.iterdump()` plus `PRAGMA user_version`.
The a2 column was then recorded with this branch; the golden test compares
fresh compatibility directly against the independently produced a1 column.
Do not regenerate expected digests just to silence a failure: changing the
pinned symphonia version or fingerprint encoding needs a version review.

The CLI retag matrix additionally requires `ffmpeg` (already a project runtime
dependency) solely to produce valid changed samples. Encoders run one at a time.
Golden values never depend on that encoder's output or version. The real app
prepared/published key writer is tested on supported formats; WAV/M4A assert
its explicit no-op and use external tags for metadata mutation coverage.
