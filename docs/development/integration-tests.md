# Integration-test binaries

The app shell groups integration tests in `src-tauri/tests/it/main.rs` and
the CLI in `crates/wreckordbox/tests/it/main.rs`, so each crate links one
ordinary integration-test binary. Each former
file is a module with its original name; its tests keep their names after
that module prefix.

Run one module through the machine-wide queue:

```sh
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo build -p wreckordbox
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p rbxport --test it service_host::
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p rbxport --test it stick_protection::
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p wreckordbox --test it library_protection::
```

The CLI must be built in the same target directory and profile before hosted
shell tests. Helpers find that prebuilt executable beside the test binary's
`deps` directory; they must never invoke Cargo inside a test.

Five shell tests deliberately keep their own binaries:

| Target | Why it needs a separate process |
| --- | --- |
| `identity` | Changes `XDG_DATA_HOME` and removes logging/backup overrides. |
| `logging` | Installs the process-global logger and changes log environment variables. |
| `omarchy_theme` | Changes the colours-file override read by the theme watcher. |
| `theme_file` | Changes both theme-file and colours-file overrides; conflicts with `omarchy_theme`. |
| `menu` | Has `harness = false` so macOS menu checks run on the main thread. |

Use `--test <target>` for these exceptions. The ignored manual probe remains
ignored inside `it`; ordinary runs must not enable it.

The CLI has no separate-process exceptions. Its shared `common` module stays
in `tests/common/` and is declared once by `it/main.rs` with a path attribute.
Existing test modules import `crate::common`; a new top-level test can still
use `mod common;` unchanged.

For a new ordinary test file, put it under `tests/it/` and declare it with
`mod <name>;` in `main.rs`. Keep Cargo's automatic test discovery enabled:
a top-level `tests/<new>.rs` arriving from another branch must still compile
and run as an independent binary until it is explicitly moved. Use `git mv`
when consolidating so concurrent edits can follow the rename.

Keep source-inspection guards rooted at `CARGO_MANIFEST_DIR`. A moved
`include_str!` or `#[path]` needs its relative path adjusted, including nested
modules and fixtures. When reorganising tests, compare the full per-crate
passed/ignored totals and individual names before and after, allowing only
the module prefix, and run the combined integration target three times with
parallel test threads. Do not silently omit a module or disable a test to
make the totals match.

## Library service

The service's environment-independent suites share
`crates/wreckordbox-service/tests/it/main.rs`. Select one through the queue:

```sh
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p wreckordbox-service --test it one_edit_gate::
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p wreckordbox-service --test it intake::
```

Ten suites retain `--test <name>`. They must not share a process with tests
that read the same environment. Their existing local mutexes cannot protect
other suites, and some deliberately leave overrides set until process exit.

| Target | Reason for isolation |
| --- | --- |
| `identity` | Changes XDG data/runtime paths and the backup-state override. |
| `dj_edit_cost` | The ignored latency benchmark changes the Sidecar path; remains isolated when explicitly enabled. |
| `dj_undo_recovery` | Changes the Sidecar path and re-executes an exact-named test with an abort fault. |
| `intensity_hosted` | Changes the Sidecar path, Intensity pause hook and test-mode environment. |
| `ledger_hosted` | Changes the Sidecar path and window Run pause environment. |
| `projection_hosted` | Changes the Sidecar path and the Projection pause environment. |
| `projection_crash` | Changes the Sidecar path and Ledger faults; re-executes its own exact-named test as the crashing child. |
| `ledger_recovery` | Changes the Sidecar path, test mode and Ledger failure injection. |
| `mutants_hosted` | Changes the Sidecar path; its mutex only serializes this file's tests. |
| `mutants_follow_hosted` | Changes the Sidecar path and Ledger/edit faults; has a file-local mutex and exact-named crash subprocess. |
| `render_stems_hosted` | Changes the Sidecar path and fake separation command. |
| `stems_hosted` | Changes the Sidecar path and fake separation command. |

When merging the pending branches `ticket/26-project-energy-prep` and
`ticket/14-11-ledger-bulk-writes`, retain new top-level `projection_hosted.rs`,
`bulk_recording.rs`, and `bulk_failures.rs` as independent tests. Projection
uses environment overrides; the bulk tests use subprocess/fault machinery
and exact-name filters, including helpers under `tests/support/`. Preserve
that support tree and its paths. Run these targets by their existing names;
do not silently omit them from Cargo discovery or blindly add them to `it`.
Any later consolidation must audit the entire helper tree and update child
filters as well as module declarations, with before/after test inventories.

## Sidecar

All ten Sidecar integration suites share
`crates/wreckordbox-sidecar/tests/it/main.rs`; none needs process isolation.

```sh
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p wreckordbox-sidecar --test it ledger::
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p wreckordbox-sidecar --test it attachment_migration::
```

The common audio fixture helper is declared once by `it/main.rs`, with
existing modules importing `crate::common`. Keep `tests/common/` and
`tests/fixtures/` in place: new top-level files can still use `mod common;`
and their original relative SQL includes. Moved modules include SQL through
`../fixtures/`. The manual attachment-scale benchmark remains ignored.

The pending Projection branch's Sidecar production changes merge normally;
Git can follow edits to the renamed tests. Any newly added top-level test
remains independently discoverable. Preserve new coverage and compare the
resulting inventory; do not force later totals back to this branch's counts.
