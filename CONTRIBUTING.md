# Contributing

`wreckordbox` is a personal, Linux-first hard fork of
[rbxport](https://github.com/chrisle/rbxport) by Chris Le, shaped around one
working DJ's setup: a background Library service, the `wreckordbox` CLI and
`wreckordbox-mcp` MCP server for agent-driven work, a Sidecar of Facts with
Projection and a durable Ledger, and an Omarchy-themed desktop window.

The repository is public under [GPL-2.0-or-later](LICENSE) so others can read,
build, or fork it. **Issues and pull requests are both welcome; you do not need to open
an issue before a pull request.** One maintainer sets the direction around their DJ
workflow and the settled architecture decisions in [`docs/adr/`](docs/adr/), so a
pull request may be declined or take a while to be reviewed. For a large change, an
issue first can save you work, but it is not required.

## Orientation

Start with:

1. [`CONTEXT.md`](CONTEXT.md): the project glossary. Use its terms (`Library`, `Stick`, `Sidecar`, `Fact`, `Projection`, `Run`, `Ledger`, `Unwind`, `Intake`, `Mutant`, and so on).
2. [`AGENTS.md`](AGENTS.md) (`CLAUDE.md`): safety rules that bind every human and agent working on the codebase.
3. [`docs/adr/`](docs/adr/): decisions already made — design within them rather than reopening them. In particular, [ADR 0003](docs/adr/0003-hard-fork-with-a-protected-core.md) keeps the player-format crates (`rbl-pdb`, `rbl-onelibrary`, `rbl-export`, `rbl-anlz`, `rbl-db`) close to upstream: add beside them rather than reshaping them.
4. [Getting started](docs/development/getting-started.md), [Architecture](docs/development/architecture.md), [Conventions](docs/development/conventions.md), and [Testing](docs/development/testing.md).

## Issues and tracking

- Historical `wbx-*` references identify development tasks. Numeric `wbx-N` identifiers through 90 correspond to this fork’s early GitHub issues; inherited three-digit issue references usually belong to upstream.
- External bug reports and questions can be opened in [GitHub Issues](https://github.com/The-Greg-O/wreckordbox/issues) using `.github/ISSUE_TEMPLATE/`. Include reproduction steps, commit hash, OS, and player model/firmware where relevant.
- **Keep everything private out of issues, commits, and tests:** no real rekordbox databases, audio files, personal filesystem paths, machine names, or personal track, playlist, or tag names.

## Branches

The integration branch is `main` (earlier fork commits used `foundation`, which has been retired into `main`; upstream's `dev` branch is not used here). Branch from `origin/main`:

```sh
git fetch origin
git switch -c fix/short-description origin/main
```

Do not include generated logs, screenshots, virtual environments, model weights, or incidental version-file edits in a branch.

## Naming in the codebase

Internal Cargo crate names (`rbl-*`, and the desktop shell's package `rbxport` / `rbxport_lib`), `package.json`'s `"name"`, and several inherited environment variables (`RBXPORT_TEST`, `RBXPORT_OPTIONS`, `RBXPORT_LOG_DIR`, `RBXPORT_WAYLAND_PRELOAD`) still keep upstream's names for now; renaming them requires a coordinated migration and is tracked separately. Do not rename them piecemeal in unrelated changes.

The built binaries already carry wreckordbox's names:

- `wreckordbox-desktop` (from package `rbxport` in `src-tauri/`)
- `wreckordbox` (headless CLI in `crates/wreckordbox/`)
- `wreckordbox-mcp` (stdio MCP server in `crates/wreckordbox-mcp/`)

## Implement and validate

- **Never write to an installed rekordbox Library or a mounted USB Stick** during development or testing. Always set `RBXPORT_TEST=1` for Rust runs, use `rbl_db::fixture::build` or the CLI test sandbox for writable Library tests, and use temporary directories in place of Sticks (never set `RBX_REAL_USB` or `RBX_USB_STICK` in automated or agent runs).
- Every command that changes the Library must go through the single edit path (`rbl-library` / `wreckordbox-service`), notify the hosting window, record in the Ledger, and honour Library Protection.
- Every new `wreckordbox` CLI subcommand must have a matching section in [`docs/agents/guide.md`](docs/agents/guide.md) (checked by `crates/wreckordbox/tests/it/agent_guide.rs`).
- Interface changes must stay within `perf-budgets.json` (`pnpm budget`) and use role tokens (`--role-*` in `src/styles/tokens.css`) rather than hard-coded greys.
- Run the targeted checks described in [Testing](docs/development/testing.md) (`cargo test`, `cargo clippy ... -- -D warnings`, `cargo fmt --check` on touched crates; `pnpm typecheck`, `pnpm lint`, `pnpm test`, `pnpm build`, `pnpm budget`, and relevant Playwright specs for frontend changes). State plainly in your pull request what passed, what you did not run, and what could not be verified on physical hardware.

Preserve upstream copyright and licence notices and submit only code you may
license under GPL-2.0-or-later. See the [fork distribution checklist](LICENSING.md#fork-distribution-checklist)
for current compliance evidence and remaining distribution checks.
