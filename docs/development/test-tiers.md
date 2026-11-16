# Test tiers

Which checks run, and when. [Testing](testing.md) says what each check proves;
this page says how validation lanes are organised so `scripts/ci/test-tiers.mjs`
and `scripts/ci/validation-plan.mjs` stay aligned.

> CI uses GitHub-hosted `ubuntu-latest` runners. The historical `release`
> tier means manual full validation, with no publishing. Windows, macOS and
> Nix lanes are disabled and excluded from planner outputs; a green gate
> does not claim those platforms were tested.

## Tiers

| Tier | Status | Trigger | Runs |
| --- | --- | --- | --- |
| `local` | active | While working, by a person or an agent | Scoped commands only: `cargo test -p <crate> [filter]`, `pnpm vitest run <paths>`, one Playwright spec. |
| `pull-request` | active | Pull request targeting `main` (including fork PRs). A newer push cancels the older run. | The lanes the path plan selects, with Rust limited to the affected packages. |
| `integration` | active | Push to `main`; runs are not cancelled | Every supported lane on Linux. |
| `nightly` | planned | Scheduled, only when `main` changed since the last green run | The cross-platform lanes: Windows, Intel macOS, Linux window. |
| `release` | active | Full-gate validation (`ci.yml` manual `workflow_dispatch` with `--all`, or local pre-merge gate) | Every supported lane unconditionally. |

Hardware checks are manual in every tier.
See [Testing](testing.md#private-suites).

## Lanes

Each lane is one output of `scripts/ci/validation-plan.mjs`. Pull-request
validation enables a lane only when a changed path selects it; full validation
enables all of them.

| Lane | Check |
| --- | --- |
| `frontend_lint` | `pnpm lint` |
| `frontend_typecheck` | `pnpm typecheck` |
| `frontend_build` | `pnpm build` |
| `frontend_unit` | `pnpm test` |
| `frontend_e2e` | `pnpm e2e` |
| `frontend_budget` | `pnpm budget` |
| `scripts` | `node --test` under `scripts/` |
| `rust` | Clippy and `cargo test` on Linux |
| `windows_rust` (disabled) | Rust compile and backup checks on Windows |
| `windows_file_drop` (disabled) | Explorer file-drop path tests |
| `linux_window` | Native `wreckordbox-desktop` window smoke under Xvfb on Linux |
| `macos_intel` (disabled) | Intel macOS Rosetta launch smoke |
| `nix_source` (disabled) | Source-built Nix package smoke on Linux |

## Rules

- **Scope local checks while iterating.** Run focused crate tests, relevant
  Vitest files, and changed Playwright specs while building, then run full
  tests/Clippy/fmt on the crates whose `src/` changed before merging to `main`.
- **A new lane is a deliberate change.** Add it to `validationKeys` in
  `scripts/ci/validation-plan.mjs`, map a path in `scripts/ci/test-tiers.test.mjs`,
  include it in `scripts/ci/test-tiers.mjs`, and list it in the table above;
  `node --test scripts/ci/test-tiers.test.mjs` enforces all four.
- **Cancel superseded PR runs.** `ci.yml` cancels in-progress validation runs on
  the same pull request when a newer push arrives.

CI builds without debug info (`CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`), and `cargo test`
skips compiling examples because `cargo clippy --all-targets` already checks
them.
