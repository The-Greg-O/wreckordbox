/**
 * Checks the gzip size of the main window's start-up code in dist/ against
 * perf-budgets.json: the entry chunk on its own, and every chunk loaded before
 * first paint together (see startup-chunks.ts). Code loaded on demand is listed
 * but not gated (DJ decision 2026-11-14). Exits unsuccessfully on overflow.
 * Run: pnpm build, then pnpm budget (which invokes this script). Read-only.
 */
import { readFileSync, readdirSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { join } from "node:path";
import { fileURLToPath, URL } from "node:url";
import { startupChunks } from "./startup-chunks";

const root = (p: string) => fileURLToPath(new URL(`../${p}`, import.meta.url));
const budgets = JSON.parse(readFileSync(root("perf-budgets.json"), "utf8")) as {
  gates: { bundle: { initialChunkKbGz: number; startupKbGz: number } };
};
const limits = budgets.gates.bundle;

// src/main.tsx awaits the main window's App chunk before its first render.
const AWAITED_BEFORE_FIRST_RENDER = ["App"];

const dir = root("dist/assets");
const code: Record<string, string> = {};
for (const f of readdirSync(dir).filter((f) => f.endsWith(".js"))) code[f] = readFileSync(join(dir, f), "utf8");
const indexHtml = readFileSync(root("dist/index.html"), "utf8");
const startup = new Set(startupChunks(indexHtml, code, AWAITED_BEFORE_FIRST_RENDER));
const entry = /<script\s+type="module"[^>]*\ssrc="\/assets\/([\w.-]+\.js)"/.exec(indexHtml)![1];

const gz = (f: string) => gzipSync(code[f]).length / 1024;
let startupKb = 0, onDemandKb = 0;
const rows: string[] = [];
for (const f of Object.keys(code).sort()) {
  const kb = gz(f);
  if (startup.has(f)) startupKb += kb; else onDemandKb += kb;
  rows.push(`  ${startup.has(f) ? "start-up " : "on demand"}  ${f}  ${kb.toFixed(1)} KB gz`);
}
const initial = gz(entry);
console.log(rows.join("\n"));
console.log(
  `entry ${initial.toFixed(1)} / ${limits.initialChunkKbGz} KB gz · start-up ${startupKb.toFixed(1)} / ${limits.startupKbGz} KB gz`
  + ` · on demand ${onDemandKb.toFixed(1)} KB gz (not gated)`,
);

const fail: string[] = [];
if (initial > limits.initialChunkKbGz) fail.push(`entry chunk ${initial.toFixed(1)} KB gz > ${limits.initialChunkKbGz}`);
if (startupKb > limits.startupKbGz) fail.push(`start-up code ${startupKb.toFixed(1)} KB gz > ${limits.startupKbGz}`);
if (fail.length) { console.error("BUNDLE BUDGET EXCEEDED:\n  " + fail.join("\n  ")); process.exit(1); }
