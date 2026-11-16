# Working on wreckordbox

Wreckordbox is a personal, Linux-first hard fork of rbxport: a rekordbox-compatible DJ library and USB export app (Tauri 2, React, Rust crates), with a background Library service, a `wreckordbox` CLI, a `wreckordbox-mcp` MCP server, and a Sidecar for what rekordbox cannot hold. Read this file before doing anything.

## Safety rules (these override everything else)

- **Never write to the user's real Library** (the rekordbox-format library under the user's home). Tests build a throwaway Library in a temporary folder with the repo's fixture builder, and always set `RBXPORT_TEST=1`.
- **Never write to a mounted USB stick.** A real Gig stick may be mounted under `/run/media/`. Do not export, sync, import, edit, format or eject anything there, and never set `RBX_REAL_USB` or `RBX_USB_STICK`. A temporary folder stands in for a Stick in every test.
- **Never unlock a Stick unless the DJ asked for that Stick, by name, to be unlocked.** `wreckordbox stick unlock <path>` works for an agent like any other client (a Stick without the Spare stick marker needs `--name` with its name); Stick Protection is the guarantee that nothing writes to a Stick, and the hold-to-confirm in the window is only an interface confirmation, so unlocking one is the DJ's decision, never an agent's.
- **Never clear a Projection barrier unless the DJ asked for that barrier to be cleared.** `wreckordbox projection acknowledge <barrier> --source dj` records the clearing as the DJ's; it is the DJ's decision, never an agent's.
- **An agent never runs `runs spills discard` or replays with an invented manifest.** Discarding accepted-write records is the DJ's explicit act. A supplied replay manifest must repeat the source, Run and kind already recorded in the Ledger; never substitute the DJ or guess missing attribution. Tests use only disposable fixture spills.
- **Never accept, reject or reopen a waiting Fact unless the DJ asked for that decision.** `wreckordbox review accept|reject|reopen ... --source dj` records it as the DJ's own act; it is the DJ's decision, never an agent's.
- **Never run a real stem separation.** It takes over twenty minutes a track on this machine. Tests use the separation engine's fake backend.
- **No `sudo`, no system package installs, no changes to the desktop or shell configuration.**
- **Do not start a long-lived app instance on the desktop.** To see the app, use the headless screenshot script (`scripts/omarchy/headless-shot.sh`).
- **Do not use the network for anything but this repository's own remote and package registries.** The Library service must stay offline (ADR 0005). Do not call any hosted model API or use any API key found in the environment.
- **Do not copy code from AZ-MODS** (GPL-3.0-only); implement its stems layout from the published document only. Wreckordbox stays GPL-2.0-or-later.
- **Nothing private in the repository:** no audio, models, virtual environments, screenshots, logs, personal paths, or the user's track names or tag data. The repository is public.

## Orientation

- `docs/agents/guide.md`: for an agent that drives wreckordbox for the DJ (through the `wreckordbox` CLI or the MCP server): every command, what it reads or changes, recipes, and which commands are the DJ's decision only. A new CLI command needs a section there (a test checks).
- `CONTEXT.md`: the project's glossary. Use its terms exactly.
- `docs/adr/`: decisions already made. Design within them; do not reopen them.
- `docs/design/`: the window's design (requirements, tokens, a render and note per screen). ADR 0007 is the structure and look; ADR 0008 is Mutants.
- `docs/development/` and `crates/README.md`: upstream's architecture, conventions and testing guides, still accurate for the parts we have not changed.
- Design history uses `wbx-*` task identifiers; public contributor guidance is in `CONTRIBUTING.md`.

## Branches

- The integration branch is `main`. All work merges there.

## Toolchain

- Versions are pinned in `mise.toml`: run tools as `mise exec -- <command>` (run `mise trust` once in a new checkout).
- Front end: install with `mise exec -- pnpm install --frozen-lockfile`. Browser specs use the mock bundle (`pnpm build:mock`), Chromium, `--workers=1 --trace off`. `pnpm dev:mock` serves the real interface against a mock backend with no Rust build.
- Rust: set `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RBXPORT_TEST=1` (and `RUSTC_WRAPPER=sccache` where installed). Bound Cargo parallelism to available memory (for example, `CARGO_BUILD_JOBS=2`).
- The reference platform is `aarch64` Linux (Asahi Linux, 16K pages) running Omarchy (Arch, Hyprland, Wayland); macOS `arm64` is also used for development.

## How to build a ticket

- Work test-first at the seams the specs agreed: the real `wreckordbox` CLI against a service on a fixture Library (the existing sandbox in the CLI crate's tests), plus a second client for events; the app shell's hosting test file where the window is involved; mock-backed browser tests and unit tests for the front end. Test behaviour a DJ or an agent could observe, not internals.
- ADR 0003: do not reshape `rbl-pdb`, `rbl-onelibrary`, `rbl-export`, `rbl-anlz` or `rbl-db`. Add beside them.
- Many workers change the same crates. Put your code in modules of its own and touch shared files (service dispatch, the CLI entry point, the app root, shared styles, workspace manifests) with the smallest possible additions.
- Every command that changes the Library goes through the single edit path, tells the hosting window, and honours Library Protection. Writing to the Sidecar is not a Library edit.
- Finish with the targeted checks below. Say plainly in your completion notes what failed, what you did not run, and what you could not verify.
- If a ticket is too large to finish well in one sitting, land the largest coherent, green part and document the remaining follow-up clearly.
- Ask the human (escalate) instead of guessing when a ticket cannot be done as specified, when it would change something an ADR decided, or when it needs hardware verification (a physical player, a real Stick, rekordbox on macOS).

## Which tests to run

Prefix the commands below with `mise exec --` and set `RBXPORT_TEST=1` for Rust.

- While building, run `cargo check` / `cargo clippy` and
  `cargo test` for one integration module or test name.
- At the end, run the full tests of crates whose `src/` changed, plus
  `cargo clippy -p <crate> --all-targets -- -D warnings` and
  `cargo fmt -p <crate> --check` (only touched files need to be clean).
- For unchanged dependent crates, use `cargo check -p <crate> --tests` and
  the specific integration modules that exercise the change, not every suite.
  For app-shell source changes, run the relevant hosted modules and guard
  tests rather than the whole shell suite.
- App-shell integration files now live in `src-tauri/tests/it/`. Select one
  with `cargo test -p rbxport --test it <module>::`; for example,
  `--test it service_host::` or `--test it stick_protection::` (command
  classification and shared-planner source guards). First build `cargo build -p wreckordbox`
  in the same target/profile for the hosted tests' prebuilt CLI.
- CLI integration files use the same layout: `cargo test -p wreckordbox --test it
  <module>::` (for example `library_protection::`).
  Its `tests/common/` stays at the original path so new top-level tests can
  still declare `mod common;`; `it/main.rs` shares it once across modules.
- Sidecar integration files also use `cargo test -p wreckordbox-sidecar --test it
  <module>::` (for example `ledger::`). Its `common/`
  helper and SQL fixtures stay under `tests/` for new standalone files.
- Service integration modules use `cargo test -p wreckordbox-service --test it
  <module>::` (for example `one_edit_gate::`). Ten
  suites that mutate process state or use crash workers remain separate;
  their names and reasons are in the integration-test guide below. Keep
  the `projection_hosted`, `bulk_recording`, and `bulk_failures` tests
  independently discoverable with their support files and child filters.
- `identity`, `logging`, `omarchy_theme`, `theme_file`, and the custom-harness
  `menu` remain separate shell targets; use `--test <name>` for them.
  Reasons and new-file/merge guidance: [integration tests](docs/development/integration-tests.md).
- Front end: typecheck, lint, relevant Vitest files and the requested browser
  specs with `mise exec -- pnpm ...`; run full suites when shared UI primitives change.
- Merge current `main` before reporting. Re-run the affected crate
  checks and your changed integration modules; include the guards if a
  command changed. Do not repeat unrelated full suites after the merge.
