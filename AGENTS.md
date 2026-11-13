# Working on wreckordbox

Wreckordbox is a personal, Linux-first hard fork of rbxport: a rekordbox-compatible DJ library and USB export app (Tauri 2, React, Rust crates), with a background Library service, a `wreckordbox` CLI, and a Sidecar for what rekordbox cannot hold. Read this file before doing anything.

## Safety rules (these override everything else)

- **Never write to the user's real Library** (the rekordbox-format library under the user's home). Tests build a throwaway Library in a temporary folder with the repo's fixture builder, and always set `RBXPORT_TEST=1`.
- **Never write to a mounted USB stick.** A real Gig stick may be mounted under `/run/media/`. Do not export, sync, import, edit, format or eject anything there, and never set `RBX_REAL_USB` or `RBX_USB_STICK`. A temporary folder stands in for a Stick in every test.
- **Never unlock a Stick unless the DJ asked for that Stick, by name, to be unlocked.** `wreckordbox stick unlock <path>` works for an agent like any other client (a Stick without the Spare stick marker needs `--name` with its name); Stick Protection is the guarantee that nothing writes to a Stick, and the hold-to-confirm in the window is only an interface confirmation, so unlocking one is the DJ's decision, never an agent's.
- **Never run a real stem separation.** It takes over twenty minutes a track on this machine. Tests use the separation engine's fake backend.
- **No `sudo`, no system package installs, no changes to the desktop or shell configuration.**
- **Do not start a long-lived app instance on the desktop.** To see the app, use the headless screenshot script (`scripts/omarchy/headless-shot.sh`).
- **Do not use the network for anything but this repository's own remote and package registries.** The Library service must stay offline (ADR 0005). Do not call any hosted model API or use any API key found in the environment.
- **Do not copy code from AZ-MODS** (GPL-3.0-only); implement its stems layout from the published document only. Wreckordbox stays GPL-2.0-or-later.
- **Nothing private in the repository:** no audio, models, virtual environments, screenshots, logs, personal paths, or the user's track names or tag data. The repository is public.

## Orientation

- `CONTEXT.md`: the project's glossary. Use its terms exactly.
- `docs/adr/`: decisions already made. Design within them; do not reopen them.
- `docs/design/`: the window's design (requirements, tokens, a render and note per screen). ADR 0007 is the structure and look; ADR 0008 is Mutants.
- `docs/development/` and `crates/README.md`: upstream's architecture, conventions and testing guides, still accurate for the parts we have not changed.
- The four specs are the epics `wbx-1` (foundation), `wbx-14` (agent capabilities), `wbx-45` (the window) and `wbx-49` (Mutants). Every ticket is a child bead with acceptance criteria. Bead `wbx-N` was GitHub issue `#N`; `#N` in a description means `wbx-N`.
- Working notes from earlier implementers live outside the repo in `/home/go/code/wreckordbox-notes/` on this machine: start with `README.md` and `IMPLEMENTER.md` there, then the file for your area (`service-protocol.md`, `edit-path.md`, `sidecar.md`, `ui-scale-and-headless-driving.md`, and others as they appear). Keep them current when you learn something a later worker needs. Where that folder's instructions about worktrees, branches or reporting differ from your orchestrator's, follow your orchestrator.

## Branches

- The integration branch is `foundation`. All work merges there. `main` is upstream's release and is not used.
- Some tickets have work in progress from an earlier implementer on a branch named `ticket/<number>-<name>` on the remote; the bead's notes say so. Start from that branch's work (merge or cherry-pick it into your own branch) instead of starting again, after judging it on its merits: its last commit may be an unreviewed "wip" snapshot that does not build.

## Toolchain

- Versions are pinned in `mise.toml`: run tools as `mise exec -- <command>` (run `mise trust` once in a new checkout).
- Heavy commands use the machine-wide queue: `/home/go/code/wreckordbox-notes/bin/heavy rust mise exec -- cargo ...` for build/test/run, `heavy check mise exec -- cargo ...` for check/clippy, and `heavy web mise exec -- pnpm ...` for install/typecheck/lint/test/build/Playwright (use the same absolute wrapper path). Never run these bare or in the background; a queued command waits its turn. `cargo fmt` runs without the queue.
- Front end: install with `pnpm install --frozen-lockfile` through the web lane. Browser specs use the mock bundle, Chromium at `/usr/bin/chromium`, `--workers=1 --trace off`. `pnpm dev:mock` serves the real interface against a mock backend with no Rust build.
- Rust: the queue sets the build-job limit, `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RBXPORT_TEST=1 RUSTC_WRAPPER=sccache SCCACHE_CACHE_SIZE=30G`. Export these for unqueued formatting too. Leave the worktree and its `target/` for review and the orchestrator's gate.
- The machine is an Apple M2 under Asahi Linux (aarch64, 16K pages, 8 cores, 15 GB RAM), running Omarchy (Arch, Hyprland, Wayland).

## How to build a ticket

- Work test-first at the seams the specs agreed: the real `wreckordbox` CLI against a service on a fixture Library (the existing sandbox in the CLI crate's tests), plus a second client for events; the app shell's hosting test file where the window is involved; mock-backed browser tests and unit tests for the front end. Test behaviour a DJ or an agent could observe, not internals.
- ADR 0003: do not reshape `rbl-pdb`, `rbl-onelibrary`, `rbl-export`, `rbl-anlz` or `rbl-db`. Add beside them.
- Many workers change the same crates. Put your code in modules of its own and touch shared files (service dispatch, the CLI entry point, the app root, shared styles, workspace manifests) with the smallest possible additions.
- Every command that changes the Library goes through the single edit path, tells the hosting window, and honours Library Protection. Writing to the Sidecar is not a Library edit.
- Finish with the targeted checks below. Say plainly in your completion notes what failed, what you did not run, and what you could not verify.
- If a ticket is too large to finish well in one sitting, land the largest coherent, green part and file the remainder as a new bead blocked appropriately.
- Ask the human (escalate) instead of guessing when a ticket cannot be done as specified, when it would change something an ADR decided, or when it needs hardware (a physical XDJ-AZ, a real Stick, rekordbox on macOS). Nothing has been verified on hardware yet.

## Which tests to run

- While building, use the check lane for `cargo check` / `cargo clippy` and
  the Rust lane for one integration module or test name.
- At the end, run the full tests of crates whose `src/` changed, plus
  `cargo clippy -p <crate> --all-targets -- -D warnings` and unqueued
  `cargo fmt -p <crate> --check` (only touched files need to be clean).
- For unchanged dependent crates, use `cargo check -p <crate> --tests` and
  the specific integration modules that exercise the change, not every suite.
  For app-shell source changes, run the relevant hosted modules and guard
  tests rather than the whole shell suite.
- App-shell integration files now live in `src-tauri/tests/it/`. Select one
  with `cargo test -p rbxport --test it <module>::`; for example,
  `--test it service_host::` or `--test it stick_protection::` (command
  classification and shared-planner source guards). These commands all use
  the Rust queue. First build `cargo build -p wreckordbox` through that queue
  in the same target/profile for the hosted tests' prebuilt CLI.
- `identity`, `logging`, `omarchy_theme`, `theme_file`, and the custom-harness
  `menu` remain separate shell targets; use `--test <name>` for them.
  Reasons and new-file/merge guidance: [integration tests](docs/development/integration-tests.md).
- Front end: typecheck, lint, relevant Vitest files and the requested browser
  specs through the web lane; leave full suites to the orchestrator.
- Merge current `foundation` before reporting. Re-run the affected crate
  checks and your changed integration modules; include the guards if a
  command changed. Do not repeat unrelated full suites after the merge.
