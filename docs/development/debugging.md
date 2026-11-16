# Debugging and local maintenance

[Documentation](../README.md) · Next: [Releases](releases.md)

Reproduce a problem at the smallest boundary first: browser/mock, Rust fixture,
CLI sandbox, loopback protocol client (`rbl-fakecdj`), desktop window, or
physical player on a Spare stick. Record which boundary failed before comparing
results from another one.

## Logs

The desktop app writes stdout and a daily log file under the OS data directory
plus `wreckordbox/logs`:

- Linux: `~/.local/share/wreckordbox/logs` (or `$XDG_DATA_HOME/wreckordbox/logs`)
- macOS: `~/Library/Application Support/wreckordbox/logs`
- Windows: `%APPDATA%\wreckordbox\logs`

The newest five daily files are retained; see `src-tauri/src/logging.rs`.

The newest 10,000 log lines since app start are also held in memory for the Log
window (`src-tauri/src/log_window.rs`, `src/views/log/`). Clicking a message or
error in the status bar opens the Log window (status messages are logged under
`rbxport::status`: errors at `warn`, others at `info`); right-clicking the app
name in the status bar opens the current log file directly.

From the command line, inspect which Library and service socket are active:

```sh
wreckordbox status          # human-readable Library path, protection state, and service info
wreckordbox --json status   # structured JSON
wreckordbox stop            # stop a standalone background service (refused if hosted by an open window)
```

### Environment variables

Several inherited environment variables (`RBXPORT_*`) and Rust log targets
(`rbxport_lib`, `rbxport::status`, `rbl_*`) keep upstream's names for now
pending a separate rename migration, while newer service and Sidecar hooks use
`WRECKORDBOX_*`:

| Variable | Effect |
| --- | --- |
| `RBXPORT_TEST` / `RB_LITE_TEST` | Refuses writes to any detected installed rekordbox Library; does not supply a fixture. |
| `RBXPORT_OPTIONS` | Points Library discovery at a custom rekordbox options file (used by CLI/service sandboxes and tests). |
| `RBXPORT_LOG_DIR` | Overrides the desktop app's log directory. |
| `LOG_LEVEL` | App-crate log level (`error`, `warn`, `info`, `debug` default, or `trace`) when `RUST_LOG` is unset. |
| `RUST_LOG` | Overrides tracing filters explicitly (for example `rbl_link=trace,rbxport_lib=info`). |
| `WRECKORDBOX_SIDECAR_DIR` | Overrides the Sidecar folder location (defaults to `wreckordbox-sidecar/` beside `master.db`). |
| `WRECKORDBOX_RUNTIME_DIR` | Overrides the service socket directory (defaults to `$XDG_RUNTIME_DIR/wreckordbox`). |
| `WRECKORDBOX_IDLE_EXIT_SECS` | Overrides the standalone service's 5-minute idle exit timeout. |
| `WRECKORDBOX_LOADING_WAIT_SECS` | Overrides the 15-second wait when a newly opened window is still loading the Library. |
| `WRECKORDBOX_THEME_FILE` / `WRECKORDBOX_THEME_COLORS` | Overrides `~/.config/wreckordbox/theme.toml` or the active Omarchy `colors.toml` palette. |
| `E2E_PORT` | Selects the Playwright server port so multiple worktrees do not collide. |

`pnpm dev` supplies a `RUST_LOG` filter in `package.json`; adjust that launch
command if you want `LOG_LEVEL` or another filter to take effect.

## Read-only diagnostics

Inspect `master.db` with a read-only `SELECT`:

```sh
mise exec -- cargo run -q -p rbl-db --example sql -- "SELECT ID, Name FROM djmdPlaylist LIMIT 5"
```

Compare `master.db` snapshots before and after one deliberate action in
rekordbox:

```sh
mise exec -- cargo run -p rbl-difftool -- record <name>
```

Inspect waveform analysis output into a separate directory without touching the
Library:

```sh
mise exec -- cargo run --release -p rbl-analysis --example waveform_probe -- AUDIO OUTPUT_DIRECTORY
```

## Common setup failures

| Symptom | Check |
| --- | --- |
| Native Linux build cannot find WebKitGTK or ALSA | Install the system packages in [Getting started](getting-started.md). |
| `pnpm dev` fails with `No valid release version` | Fetch git tags (`git fetch --tags`); `scripts/sync-version.mjs` looks for merged `v*` tags. |
| Playwright cannot launch a browser | Run `mise exec -- pnpm exec playwright install chromium`. |
| Two worktrees collide on the Playwright server port | Set `E2E_PORT` per worktree. |
| A write test is refused with `write_refused` or `library_protected` | Ensure the test builds a disposable fixture Library and turns protection off only on that fixture — never on a real Library. |
| `wreckordbox-stick-protection` or CLI fails to compile on macOS | Run Linux-specific crate tests inside an `aarch64` Linux environment; Linux is the reference platform. |

## Cleanup

Preview local build-artifact cleanup with `pnpm clean -- --dry-run`, then run
`pnpm clean` to remove generated build and test output from the current
checkout.

Optional flags extend it: `--dependencies` removes `node_modules`; `--app-data`
removes app caches and abandoned partial backups (close the app first);
`--git` prunes missing worktrees and local branches already merged into `HEAD`
(preserving `main`, `dev`, the current branch, and checked-out worktree
branches). Completed backups, preferences, logs, recovery journals, and
rekordbox Libraries are never deleted by `pnpm clean`.
