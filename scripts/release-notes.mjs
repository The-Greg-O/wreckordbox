/**
 * Validates structured release-notes.json and renders it as Markdown.
 * Exports helpers for preflight and tests.
 * Run: node scripts/release-notes.mjs validate FILE [VERSION] or
 * markdown FILE OUTPUT. The Markdown mode writes its output file.
 */
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/;
const DATE = /^\d{4}-\d{2}-\d{2}$/;
const CHANGE = /^\((New|Fixed|Improved)\)\s+\S/;

export function readReleaseNotes(path) {
  const notes = JSON.parse(readFileSync(path, "utf8"));
  if (!Array.isArray(notes) || notes.length === 0) {
    throw new Error("release notes must be a non-empty JSON array");
  }
  const versions = new Set();
  for (const [index, note] of notes.entries()) {
    if (!note || typeof note !== "object" || Array.isArray(note)) {
      throw new Error(`release ${index + 1} must be an object`);
    }
    if (typeof note.version !== "string" || !VERSION.test(note.version)) {
      throw new Error(`release ${index + 1} has an invalid version`);
    }
    if (versions.has(note.version)) throw new Error(`duplicate release ${note.version}`);
    versions.add(note.version);
    if (typeof note.date !== "string" || !DATE.test(note.date)) {
      throw new Error(`release ${note.version} has an invalid date`);
    }
    if (!Array.isArray(note.changes) || note.changes.length === 0) {
      throw new Error(`release ${note.version} has no changes`);
    }
    for (const change of note.changes) {
      if (typeof change !== "string" || !CHANGE.test(change)) {
        throw new Error(`release ${note.version} has an invalid change: ${String(change)}`);
      }
    }
  }
  return notes;
}

export function releaseForVersion(notes, version) {
  const release = notes.find((note) => note.version === version);
  if (!release) throw new Error(`release-notes.json has no entry for ${version}`);
  return release;
}

function groupedChanges(release) {
  const groups = new Map();
  for (const raw of release.changes) {
    const match = /^\((New|Fixed|Improved)\)\s+(.+)$/.exec(raw);
    if (!match) continue;
    const [, label, text] = match;
    const values = groups.get(label) ?? [];
    values.push(text);
    groups.set(label, values);
  }
  return groups;
}

function releaseBody(release) {
  const sections = [];
  for (const [label, changes] of groupedChanges(release)) {
    const heading = label === "New" ? "Added" : label;
    sections.push(`### ${heading}\n${changes.map((change) => `- ${change}`).join("\n")}`);
  }
  return sections.join("\n\n");
}

export function releaseMarkdown(notes) {
  return notes.map((release) =>
    `## [${release.version}] — ${release.date}\n\n${releaseBody(release)}`,
  ).join("\n\n");
}

function usage() {
  throw new Error("Usage: release-notes.mjs validate FILE [VERSION] | markdown FILE OUTPUT");
}

function main(args) {
  const [command, file, ...rest] = args;
  if (!command || !file) usage();
  const notes = readReleaseNotes(file);
  if (command === "validate") {
    if (rest[0]) {
      releaseForVersion(notes, rest[0]);
      if (notes[0].version !== rest[0]) {
        throw new Error(`release ${rest[0]} must be the first entry in release-notes.json`);
      }
    }
    console.log(`Validated ${notes.length} release-note ${notes.length === 1 ? "entry" : "entries"}.`);
    return;
  }
  if (command === "markdown") {
    const [output] = rest;
    if (!output) usage();
    writeFileSync(output, `${releaseMarkdown(notes)}\n`);
    return;
  }
  usage();
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2));
}
