# GitHub Actions layout

This fork keeps only read-only validation workflows and issue-platform labelling.
Upstream's release, R2 CDN publication, Microsoft Store submission, installer
signing, and scheduled Nix AppImage-seeding workflows have been removed so
nothing in this repository can publish to upstream's channels or mutate branches.

Note: the validation jobs declare upstream's `[self-hosted, ...]` runner labels
and are only active if matching self-hosted runners are registered on this
repository.

## Entry points

- `ci.yml` runs on same-repository pull requests targeting `main` and on manual
  `workflow_dispatch`. It uses `scripts/ci/validation-plan.mjs` to classify
  changed files and requests only the matching validation lanes (or every lane
  on a manual run), with `contents: read` permissions only.
- `label-issue-platform.yml` applies the `macos`, `windows`, or `linux` label
  from `.github/ISSUE_TEMPLATE/bug.yml`.

## Reusable validation workflows

- `validation.yml` coordinates the aggregate validation gate.
- `validate-rust.yml` runs Clippy, unit/integration/doc tests, and (on full
  runs) the Rust 1.89 MSRV check. On pull requests, `scripts/ci/affected-rust-packages.mjs`
  narrows checks to changed workspace crates and their reverse dependents.
- `validate-frontend.yml` selects lint, typecheck, build, unit, Playwright, and
  bundle-budget checks.
- `validate-linux.yml` proves the Tauri shell (`wreckordbox-desktop`) opens a
  native Linux window under Xvfb.
- `validate-macos.yml` checks Intel macOS launch under Rosetta and (when
  `validate_desktop` is enabled) Apple Silicon desktop scenarios against a
  disposable SQLCipher fixture.
- `validate-windows.yml` checks Windows-native compilation, backup locking, and
  file-drop path handling.
- `validate-nix.yml` builds and smoke-tests the source Nix package via
  `.github/actions/nix-smoke`.
- `validate-scripts.yml` runs the Node and Python script unit tests.
