# Integration-test binaries

The app shell groups integration tests in `src-tauri/tests/it/main.rs` so a
change to the shell links one ordinary integration-test binary. Each former
file is a module with its original name; its tests keep their names after
that module prefix.

Run one module through the machine-wide queue:

```sh
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo build -p wreckordbox
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p rbxport --test it service_host::
/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo test -p rbxport --test it stick_protection::
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
