# Ledger failure paths (wbx-14.6)

Queries do not recover, open writers, create Runs, repair manifests, or delete
files. `runs.list` succeeds when accepted bulk results have no Run yet: it adds
`pending_recording` and a `recovery` inventory to the recorded Runs. `runs.show`
also includes pending ordinary append failures and bulk results for its Run.
Pending pages and committed-entry pages use the same offset/limit independently.
Malformed files remain visible as diagnostics, not as empty successful results.

## Local spill commands

- `runs.spills.list {offset?, limit?}` lists directories, original manifests,
  errors and file/count summaries. CLI: `wreckordbox runs spills list`.
- `runs.spills.show {id, offset?, limit?}` adds a page of accepted records.
  CLI: `wreckordbox runs spills show ID --limit 128`.
- `runs.spills.replay {id, manifest?}` explicitly replays completed work. With a
  missing or damaged manifest, supply the original attribution and operation
  metadata; no DJ source is guessed. CLI: `wreckordbox runs spills replay ID
  --manifest original-manifest.json`. Existing valid manifests cannot be
  replaced. Analysis, LINK and Round-trip retain their fixed tool sources.
- `runs.spills.discard {id, source, reason}` requires an explicit DJ source,
  even in-process; tool/agent sources are refused. CLI: `wreckordbox runs spills
  discard ID --source dj --reason '...'`. This discards recording evidence,
  not the accepted Library changes. A nonreversible `ledger_spill_discard`
  entry records the DJ's decision, original file summary and reason first.

The JSON manifest has `intent`, `run` (nullable integer ids), `source`
(`{kind,name}`), `description`, `kind` (`analysis`, `import`, `link`,
`round_trip`) and `complete`. Use original identifiers where they are known.
A missing manifest cannot establish provenance by itself: inspect the retained
results and original Run/intent before supplying replacement metadata.

Directory ids cannot escape local recovery storage. Symlinks and nonregular
files are refused. Batches are bounded to 128 records and 16 MiB on read. A
short-lived in-process claim prevents recovery or discard of an active
operation; its mutex is released before filesystem/Sidecar I/O. Other Keeper
state is likewise copied/updated outside this command's I/O.

Discard persists a receipt with stable entry/Run identity before append. Failed
append retains all accepted bytes. Explicit retry reuses the receipt. Startup
and reads preserve a pending discard; only another explicit DJ discard finishes
it. File removals are synced before removing the receipt. Replay preserves
stable ids, original source and one Run, and reports failures. Resolving a spill
removes its live diagnostic; errors are not accumulated forever in Keeper state.

## Other review findings

- Native commit observation uses a separate read-only connection's SQLite
  `data_version`, under the existing edit gate. `total_changes` includes rolled
  back statements and cannot prove a write committed. A committed prefix still
  counts when a later transaction fails. A known refusal is durably settled
  immediately; visibility errors leave the intent uncertain.
- The permanent core wrapper and unrecorded/permanent shell wrappers retain the
  refresh result, record/invalidate history, then return that result's error.
- Each accepted cue-conversion slice records immediately with `part_of` linking
  to its first slice; its exact preceding history revision joins one Undo entry.
  Intervening history is never swallowed. Notifications are coalesced at the end.
- Ordinary spill replay creates a missing recovery Run in the same transaction
  as its entry, Facts and Mutant consequences. Stable-id replay creates no Run.
- Normal binaries compile no environment fault parser. Tests enable the service's
  `test-faults` feature through dev-dependencies. A manually prebuilt CLI for
  fault/crash tests needs `--features wreckordbox-service/test-faults`. The ignored
  `fault_boundary` test instead takes `WRECKORDBOX_NORMAL_BINARY`, a separately
  prebuilt normal executable, and verifies an accepted write survives the fault
  environment and is recorded.

## Existing Sidecar history

Mutant changes, Decision questions with a Run, and answers with a Run append a
Ledger snapshot in their existing transaction. Inspection counts this one
representation, not both native and Ledger rows. Sources and original native
ids remain in the snapshot. They do not become DJ Facts or native Library edits.

`native_history::catch_up` is a data migration, on the writer's open only: batches
of at most 128 old native rows and a per-table cursor commit together. A failure
rolls the batch/cursor back. Reopening cannot duplicate it. It changes no schema;
version 9 and the backup schema contract remain valid. Read-only readers never
run it. Questions/answers without a Run remain outside a Run's history.

New records use the shared Ledger sequence. Old native records never had one:
`historical_order: true` marks them, and inspection places them by their retained
timestamps while preserving the known sequence of existing Ledger entries and
Facts. Equal-time order among historical records is deterministic, not proof of
an order the old store never retained. Original native tables remain untouched.
