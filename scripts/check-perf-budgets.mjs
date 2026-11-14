#!/usr/bin/env node

/**
 * Validates perf-budgets.json: its schema, positive limits, enforcement
 * settings, and recorded browser baselines against their gates.
 * Run: pnpm budget (also checks built bundle sizes). Read-only; this script
 * does not collect fresh performance measurements.
 */
import { readFile } from "node:fs/promises";

const path = new URL("../perf-budgets.json", import.meta.url);
const budgets = JSON.parse(await readFile(path, "utf8"));

function object(value, name) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${name} must be an object`);
  }
  return value;
}

function exactKeys(value, expected, name) {
  const actual = Object.keys(object(value, name)).sort();
  const wanted = [...expected].sort();
  if (actual.join("\n") !== wanted.join("\n")) {
    throw new Error(`${name} keys must be ${wanted.join(", ")}; found ${actual.join(", ")}`);
  }
}

function positive(value, name) {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    throw new Error(`${name} must be a positive number`);
  }
  return value;
}

exactKeys(budgets, ["$schemaVersion", "$comment", "baseline", "gates", "manualTargets", "enforcement"], "root");
if (budgets.$schemaVersion !== 2) throw new Error("$schemaVersion must be 2");
if (typeof budgets.$comment !== "string" || budgets.$comment.length === 0) throw new Error("$comment must describe the budget policy");

exactKeys(budgets.baseline, ["date", "sourceCommit", "environment", "bundle", "browser", "startup"], "baseline");
if (!/^\d{4}-\d{2}-\d{2}$/.test(budgets.baseline.date)) throw new Error("baseline.date must be YYYY-MM-DD");
if (!/^[0-9a-f]{7,40}$/.test(budgets.baseline.sourceCommit)) throw new Error("baseline.sourceCommit must be a Git commit");
if (typeof budgets.baseline.environment !== "string" || budgets.baseline.environment.length === 0) {
  throw new Error("baseline.environment must describe the measurement environment");
}

exactKeys(budgets.gates, ["bundle", "browser", "ipc"], "gates");
exactKeys(budgets.gates.bundle, ["initialChunkKbGz", "startupKbGz"], "gates.bundle");
exactKeys(budgets.gates.browser, ["firstRowsMs", "deckFrameCostMs", "scrollFrameCostMs"], "gates.browser");
exactKeys(budgets.gates.ipc, ["responseKb"], "gates.ipc");
for (const metric of ["firstRowsMs", "deckFrameCostMs", "scrollFrameCostMs"]) {
  exactKeys(budgets.gates.browser[metric], ["chromium", "webkit"], `gates.browser.${metric}`);
  for (const browser of ["chromium", "webkit"]) {
    positive(budgets.gates.browser[metric][browser], `gates.browser.${metric}.${browser}`);
  }
}

// The 2026-10-04 bundle baseline is kept as history. Its total is no longer
// gated (DJ decision 2026-11-14: only start-up code is), so it is only checked
// to be a measurement.
const measuredBundle = object(budgets.baseline?.bundle, "baseline.bundle");
exactKeys(measuredBundle, ["initialChunkKbGz", "totalKbGz"], "baseline.bundle");
positive(measuredBundle.totalKbGz, "baseline.bundle.totalKbGz");
if (positive(measuredBundle.initialChunkKbGz, "baseline.bundle.initialChunkKbGz")
  > positive(budgets.gates.bundle.initialChunkKbGz, "gates.bundle.initialChunkKbGz")) {
  throw new Error("baseline.bundle.initialChunkKbGz exceeds its gate");
}

// Start-up code: what the main window loads before first paint.
const startup = object(budgets.baseline?.startup, "baseline.startup");
exactKeys(startup, ["date", "sourceCommit", "environment", "startupKbGz", "chunks"], "baseline.startup");
if (!/^\d{4}-\d{2}-\d{2}$/.test(startup.date)) throw new Error("baseline.startup.date must be YYYY-MM-DD");
if (!/^[0-9a-f]{7,40}$/.test(startup.sourceCommit)) throw new Error("baseline.startup.sourceCommit must be a Git commit");
if (typeof startup.environment !== "string" || startup.environment.length === 0) {
  throw new Error("baseline.startup.environment must describe the measurement environment");
}
const startupChunks = object(startup.chunks, "baseline.startup.chunks");
let startupSum = 0;
for (const [name, kb] of Object.entries(startupChunks)) startupSum += positive(kb, `baseline.startup.chunks.${name}`);
const startupMeasured = positive(startup.startupKbGz, "baseline.startup.startupKbGz");
if (Math.abs(startupSum - startupMeasured) > 0.5) {
  throw new Error(`baseline.startup.chunks add up to ${startupSum.toFixed(1)}, not startupKbGz ${startupMeasured}`);
}
if (startupMeasured > positive(budgets.gates.bundle.startupKbGz, "gates.bundle.startupKbGz")) {
  throw new Error("baseline.startup.startupKbGz exceeds its gate");
}

const measuredBrowsers = object(budgets.baseline?.browser, "baseline.browser");
exactKeys(measuredBrowsers, ["chromium", "webkit"], "baseline.browser");
for (const browser of ["chromium", "webkit"]) {
  const measured = object(measuredBrowsers[browser], `baseline.browser.${browser}`);
  exactKeys(
    measured,
    ["firstRowsMedianMs", "firstRowsMaxMs", "deckFrameCostMaxMs", "scrollFrameCostMaxMs"],
    `baseline.browser.${browser}`,
  );
  positive(measured.firstRowsMedianMs, `baseline.browser.${browser}.firstRowsMedianMs`);
  positive(measured.firstRowsMaxMs, `baseline.browser.${browser}.firstRowsMaxMs`);
  for (const key of ["deckFrameCostMaxMs", "scrollFrameCostMaxMs"]) {
    if (typeof measured[key] !== "number" || !Number.isFinite(measured[key]) || measured[key] < 0) {
      throw new Error(`baseline.browser.${browser}.${key} must be a non-negative number`);
    }
  }
  if (measured.firstRowsMaxMs > budgets.gates.browser.firstRowsMs[browser]) {
    throw new Error(`baseline.browser.${browser}.firstRowsMaxMs exceeds its gate`);
  }
  if (measured.deckFrameCostMaxMs > budgets.gates.browser.deckFrameCostMs[browser]) {
    throw new Error(`baseline.browser.${browser}.deckFrameCostMaxMs exceeds its gate`);
  }
  if (measured.scrollFrameCostMaxMs > budgets.gates.browser.scrollFrameCostMs[browser]) {
    throw new Error(`baseline.browser.${browser}.scrollFrameCostMaxMs exceeds its gate`);
  }
}

positive(budgets.gates.ipc.responseKb, "gates.ipc.responseKb");

exactKeys(budgets.manualTargets, ["nativeStartup", "interaction", "idle", "player"], "manualTargets");
exactKeys(budgets.manualTargets.nativeStartup, ["chromePaintedMsMac", "chromePaintedMsWin"], "manualTargets.nativeStartup");
exactKeys(budgets.manualTargets.interaction, ["sortRepaintMs", "searchKeystrokeP95Ms", "fetchRowsMs"], "manualTargets.interaction");
exactKeys(budgets.manualTargets.idle, ["cpuPercent", "rustRssMb", "webviewRssMb"], "manualTargets.idle");
exactKeys(
  budgets.manualTargets.player,
  ["deckLoadToAudioMs", "scrubToVisibleMs", "deckTickHz", "waveformRedrawsPerSecond"],
  "manualTargets.player",
);
for (const [group, values] of Object.entries(budgets.manualTargets)) {
  for (const [name, value] of Object.entries(values)) positive(value, `manualTargets.${group}.${name}`);
}

exactKeys(budgets.enforcement, ["ci", "local", "measurement"], "enforcement");
exactKeys(budgets.enforcement.ci, ["bundle", "firstRows", "ipcResponse"], "enforcement.ci");
exactKeys(budgets.enforcement.local, ["deckFrames", "scrollFrames"], "enforcement.local");
for (const [scope, values] of Object.entries({ ci: budgets.enforcement.ci, local: budgets.enforcement.local })) {
  for (const [name, value] of Object.entries(values)) {
    if (typeof value !== "string" || value.length === 0) throw new Error(`enforcement.${scope}.${name} must name its check`);
  }
}
if (typeof budgets.enforcement.measurement !== "string" || budgets.enforcement.measurement.length === 0) {
  throw new Error("enforcement.measurement must name the rebaseline command");
}

console.log("Performance budget schema and recorded baseline are valid.");
