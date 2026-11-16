# GitHub Actions layout

`ci.yml` validates pull requests targeting `main` (including fork PRs), pushes
to `main`, and manual dispatches. All jobs use ephemeral `ubuntu-latest`
runners, read-only permissions and no secrets. Checkouts do not persist token
credentials. External actions are pinned to verified commit SHAs; comments
name the tags resolved when updating them.

PRs select lanes by changed paths. Pushes and manual runs run every supported
lane. Superseded PR runs are cancelled; main-branch runs are not.

- `validate-rust.yml`: formatting, Clippy, unit/integration/doc tests, plus the
  declared MSRV on full runs. Hosted shell tests build the CLI first.
- `validate-frontend.yml`: lint, typecheck, build, budget, Vitest and Chromium
  mock E2E tests with one browser worker. WebKit is not in this gate.
- `validate-scripts.yml`: dependency-free Node script tests.
- `validate-linux.yml`: native Tauri window smoke under Xvfb.
- `validation.yml`: coordinates lanes and reports `Validation complete`,
  failing when a dependency fails or is cancelled. Docs-only PRs may skip lanes.

macOS Rosetta/TCC automation and Windows persistent-host/vcpkg checks have been
removed: neither was a cheap portable hosted lane. Nix smoke is also off until
its source hashes and hosted build cost are validated. The planner retains
historical path classification but excludes these disabled lanes from outputs.
These platforms are not covered by a green gate.

Release/signing/CDN/store workflows, persistent-runner cleanup and issue-label
write automation are removed. Nothing here publishes, labels issues or mutates
branches. Enabling fork Actions and configuring branch rules are separate
repository-owner actions; first verify a hosted run and its exact check names.
