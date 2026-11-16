import assert from "node:assert/strict";
import test from "node:test";

import { planValidation, validationKeys } from "./validation-plan.mjs";

function enabled(...paths) {
  const plan = planValidation(paths);
  return validationKeys.filter(key => plan[key]);
}

test("documentation-only changes require no expensive validation lane", () => {
  assert.deepEqual(enabled("README.md", "docs/testing.md", ".github/workflows/README.md"), []);
});

test("a database library change uses focused Rust checks", () => {
  assert.deepEqual(enabled("crates/rbl-db/src/export.rs"), ["rust"]);
});

test("desktop shell changes use focused Rust checks", () => {
  assert.deepEqual(enabled("src-tauri/src/lib.rs"), ["rust"]);
});

test("desktop-related PR changes leave native regressions to deployment", () => {
  for (const path of [
    "src-tauri/src/cues.rs",
    "crates/rbl-db/src/locate.rs",
    "src/views/player/useCueWriter.ts",
    "src/ipc/client.ts",
    "src/app/App.tsx",
    "src/views/settings/BackupsPane.tsx",
    "crates/rbl-export/src/lib.rs",
  ]) {
    assert.ok(!enabled(path).includes("macos_intel"), path);
  }
  assert.deepEqual(enabled("scripts/desktop/run.mjs"), ["scripts"]);
});

test("Windows installer resources retain Linux checks (Windows is disabled)", () => {
  assert.deepEqual(enabled("src-tauri/windows/test-common-controls-v6.manifest"), ["rust"]);
});

test("Windows-only desktop Rust retains Linux checks (Windows is disabled)", () => {
  assert.deepEqual(enabled("src-tauri/src/file_drop.rs"), ["rust"]);
  assert.deepEqual(enabled("src-tauri/Cargo.toml"), ["rust"]);
});

test("the library's Windows file-index code retains Linux checks (Windows is disabled)", () => {
  assert.deepEqual(enabled("crates/rbl-db/src/file_id.rs"), ["rust"]);
  assert.deepEqual(enabled("crates/rbl-db/Cargo.toml"), ["rust"]);
});

test("USB eject's Windows code retains Linux checks (Windows is disabled)", () => {
  assert.deepEqual(enabled("crates/rbl-devices/src/eject.rs"), ["rust"]);
  assert.deepEqual(enabled("crates/rbl-devices/src/eject/windows.rs"), ["rust"]);
  assert.deepEqual(enabled("crates/rbl-devices/Cargo.toml"), ["rust"]);
  assert.deepEqual(enabled("crates/rbl-devices/src/lib.rs"), ["rust"]);
});

test("backup changes retain Linux checks (Windows is disabled)", () => {
  assert.deepEqual(enabled("src-tauri/src/backups.rs"), ["rust"]);
  assert.deepEqual(enabled("crates/rbl-backup/src/restore.rs"), ["rust"]);
});

test("frontend test-only changes skip browser and bundle checks", () => {
  assert.deepEqual(enabled("src/lib/cues.test.ts"), [
    "frontend_lint",
    "frontend_typecheck",
    "frontend_unit",
  ]);
});

test("frontend source changes use the fast frontend checks", () => {
  assert.deepEqual(enabled("src/views/player/useGridEditor.ts"), [
    "frontend_lint",
    "frontend_typecheck",
    "frontend_unit",
  ]);
});

test("Playwright-only changes run lint, build, and browser tests", () => {
  assert.deepEqual(enabled("e2e/grid-edit.spec.ts"), [
    "frontend_lint",
    "frontend_typecheck",
    "frontend_build",
    "frontend_e2e",
  ]);
});

test("file-drop bridge changes retain Linux checks (Windows is disabled)", () => {
  assert.deepEqual(enabled("src/ipc/file-drop.ts"), [
    "frontend_lint",
    "frontend_typecheck",
    "frontend_unit",
  ]);
});

test("release scripts use the focused Node script suite", () => {
  assert.deepEqual(enabled("scripts/generate-release-notes.mjs"), ["scripts"]);
});

test("the Vitest-based cleanup script also runs frontend unit tests", () => {
  assert.deepEqual(enabled("scripts/cleanup.mjs"), ["frontend_unit", "scripts"]);
});

test("packaging action changes defer expensive runtime checks to deploy", () => {
  assert.deepEqual(enabled(".github/actions/package-installer/action.yml"), ["scripts"]);
});

test("the AppImage GTK plugin runs the Node script suite that tests its hook", () => {
  assert.deepEqual(enabled("packaging/appimage/linuxdeploy-plugin-gtk.sh"), ["scripts"]);
});

test("Nix packaging has no enabled build lane", () => {
  assert.deepEqual(enabled("flake.nix"), []);
  assert.deepEqual(enabled("flake.lock"), []);
  assert.deepEqual(enabled("packaging/nix/pin.json"), []);
  assert.deepEqual(enabled("packaging/nix/source.nix"), []);
  assert.deepEqual(enabled(".github/workflows/validate-nix.yml"), ["scripts"]);
  assert.deepEqual(enabled(".github/workflows/nix.yml"), ["scripts"]);
  assert.deepEqual(enabled(".github/workflows/nix-update.yml"), ["scripts"]);
  assert.deepEqual(enabled(".github/actions/nix-smoke/action.yml"), ["scripts"]);
});

test("source and dependency changes leave the source smoke to release", () => {
  assert.deepEqual(enabled("Cargo.lock"), ["rust"]);
  assert.deepEqual(enabled("pnpm-lock.yaml"), [
    "frontend_lint",
    "frontend_typecheck",
    "frontend_build",
    "frontend_unit",
    "scripts",
  ]);
  assert.deepEqual(enabled("package.json"), [
    "frontend_lint",
    "frontend_typecheck",
    "frontend_build",
    "frontend_unit",
    "scripts",
  ]);
  assert.deepEqual(enabled("src/lib/cues.ts"), [
    "frontend_lint",
    "frontend_typecheck",
    "frontend_unit",
  ]);
  assert.deepEqual(enabled("public/icon.png"), ["frontend_build"]);
});

test("a lane workflow change exercises only that lane on the PR", () => {
  assert.deepEqual(enabled(".github/workflows/validate-windows.yml"), [
    "scripts",
  ]);
});

test("manual validation enables every lane", () => {
  assert.deepEqual(
    validationKeys.filter(key => planValidation([], { all: true })[key]),
    validationKeys,
  );
});
