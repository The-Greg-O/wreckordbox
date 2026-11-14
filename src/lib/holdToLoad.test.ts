import { describe, expect, it } from "vitest";

import {
  holdMs,
  IDLE,
  loadStep,
  progressLine,
  progressPercent,
  fromServiceProgress,
  type LoadMachine,
  type LoadProgress,
} from "./holdToLoad";
import type { LoadOutcome } from "./stickLoad";

const pointer = { pointer: 1 };
const key = { key: "l" };

const outcome = (over: Partial<LoadOutcome> = {}): LoadOutcome => ({
  stick: {
    path: "/media/GIG", name: "GIG", kind: "gig", protected: false, unfinishedWrite: false, lastLoaded: null,
    freeBytes: null, totalBytes: null, tracks: null, playlists: null,
  },
  outcome: "loaded", error: null,
  written: { tracks: ["A"], playlists: ["P"] }, notWritten: { tracks: [], playlists: [] },
  unchanged: { tracks: [], playlists: [] }, holds: { tracks: 1, playlists: 1 }, nativeStems: [], reportId: null,
  ...over,
});

const progress = (over: Partial<LoadProgress> = {}): LoadProgress => ({
  stage: "copying", done: 3, total: 10, track: { id: "7", title: "Signal Fire" }, ...over,
});

/** Runs events from `start`, answering the state after each. */
function run(start: LoadMachine, ...events: Parameters<typeof loadStep>[1][]): LoadMachine {
  return events.reduce(loadStep, start);
}

describe("the hold", () => {
  it("is twice as long for a Gig stick as for a Spare stick", () => {
    expect(holdMs("spare")).toBe(1000);
    expect(holdMs("gig")).toBe(2000);
    expect(holdMs("gig")).toBeGreaterThan(holdMs("spare"));
  });

  it("a short press does nothing: released before it is full, it is idle again and nothing starts", () => {
    const pressed = run(IDLE, { type: "press", holder: pointer, at: 0, kind: "spare" });
    expect(pressed.state).toBe("holding");
    expect(run(pressed, { type: "tick", at: 400 }).state).toBe("holding");
    expect(run(pressed, { type: "tick", at: 400 }, { type: "release", holder: pointer })).toEqual(IDLE);
  });

  it("a full hold starts the load", () => {
    const held = run(IDLE, { type: "press", holder: key, at: 100, kind: "spare" }, { type: "tick", at: 1100 });
    expect(held).toEqual({ state: "loading", shrunk: false, progress: null });
  });

  it("a Gig stick's hold is not full after a Spare stick's time", () => {
    const gig = run(IDLE, { type: "press", holder: pointer, at: 0, kind: "gig" }, { type: "tick", at: 1500 });
    expect(gig.state).toBe("holding");
    expect(run(gig, { type: "tick", at: 2000 }).state).toBe("loading");
  });

  it("only the input that started the hold can end it; another letting go changes nothing", () => {
    const pressed = run(IDLE, { type: "press", holder: pointer, at: 0, kind: "spare" });
    expect(run(pressed, { type: "release", holder: key }).state).toBe("holding");
    expect(run(pressed, { type: "release", holder: { pointer: 2 } }).state).toBe("holding");
    expect(run(pressed, { type: "release", holder: { pointer: 1 } })).toEqual(IDLE);
  });

  it("an interruption (focus lost, window hidden) cancels the hold", () => {
    const pressed = run(IDLE, { type: "press", holder: key, at: 0, kind: "gig" });
    expect(run(pressed, { type: "interrupt" })).toEqual(IDLE);
  });

  it("a second press while holding does not restart the clock", () => {
    const pressed = run(IDLE, { type: "press", holder: pointer, at: 0, kind: "spare" }, { type: "press", holder: key, at: 900, kind: "spare" });
    expect(run(pressed, { type: "tick", at: 1000 }).state).toBe("loading");
  });

  it("a release after the hold is full does not cancel the load it started", () => {
    const loading = run(IDLE, { type: "press", holder: pointer, at: 0, kind: "spare" }, { type: "tick", at: 1000 }, { type: "release", holder: pointer });
    expect(loading.state).toBe("loading");
  });
});

describe("the load", () => {
  const loading: LoadMachine = { state: "loading", shrunk: false, progress: null };

  it("follows progress, and keeps it while shrunk to the status line and brought back", () => {
    const moved = run(loading, { type: "progress", progress: progress() });
    expect(moved).toEqual({ state: "loading", shrunk: false, progress: progress() });
    const shrunk = run(moved, { type: "shrink" });
    expect(shrunk).toEqual({ state: "loading", shrunk: true, progress: progress() });
    const later = run(shrunk, { type: "progress", progress: progress({ done: 4 }) });
    expect(later).toEqual({ state: "loading", shrunk: true, progress: progress({ done: 4 }) });
    expect(run(later, { type: "restore" })).toEqual({ state: "loading", shrunk: false, progress: progress({ done: 4 }) });
  });

  it("ignores a press while loading: no second load", () => {
    expect(run(loading, { type: "press", holder: pointer, at: 0, kind: "spare" }, { type: "tick", at: 5000 })).toEqual(loading);
  });

  it("ends finished when everything planned was written", () => {
    const done = run(loading, { type: "finished", outcome: outcome() });
    expect(done).toEqual({ state: "finished", outcome: outcome() });
  });

  it("ends failed, with the report, when the load was incomplete or failed", () => {
    const incomplete = outcome({ outcome: "incomplete", notWritten: { tracks: ["B"], playlists: [] } });
    expect(run(loading, { type: "finished", outcome: incomplete })).toEqual({ state: "failed", outcome: incomplete, error: null });
    const failed = outcome({ outcome: "failed", error: "removed" });
    expect(run(loading, { type: "finished", outcome: failed })).toEqual({ state: "failed", outcome: failed, error: null });
  });

  it("ends failed with the reason when the load was refused or broke with no report", () => {
    expect(run({ ...loading, shrunk: true }, { type: "failed", error: "not confirmed" })).toEqual({ state: "failed", outcome: null, error: "not confirmed" });
  });

  it("a finished or failed load is dismissed back to idle; a running one is not", () => {
    expect(run({ state: "finished", outcome: outcome() }, { type: "dismiss" })).toEqual(IDLE);
    expect(run({ state: "failed", outcome: null, error: "x" }, { type: "dismiss" })).toEqual(IDLE);
    expect(run(loading, { type: "dismiss" })).toEqual(loading);
  });

  it("shrinking is only for a running load", () => {
    expect(run(IDLE, { type: "shrink" })).toEqual(IDLE);
    expect(run({ state: "finished", outcome: outcome() }, { type: "shrink" })).toEqual({ state: "finished", outcome: outcome() });
  });

  it("progress before the load or after it ends changes nothing", () => {
    expect(run(IDLE, { type: "progress", progress: progress() })).toEqual(IDLE);
    expect(run({ state: "finished", outcome: outcome() }, { type: "progress", progress: progress() })).toEqual({ state: "finished", outcome: outcome() });
  });
});

describe("what the progress says", () => {
  it("reads the service's event", () => {
    expect(fromServiceProgress({ stick: "/media/GIG", stage: "copying", done: 2, total: 5, track: { id: "9", title: "Kiln" } }))
      .toEqual({ stick: "/media/GIG", progress: { stage: "copying", done: 2, total: 5, track: { id: "9", title: "Kiln" } } });
    expect(fromServiceProgress({ stick: "/media/GIG", stage: "database", done: 0, total: 0, track: null }))
      .toEqual({ stick: "/media/GIG", progress: { stage: "database", done: 0, total: 0, track: null } });
    expect(fromServiceProgress({ stage: "copying" })).toBeNull();
  });

  it("a percentage that never runs past 100 and is unknown before the first step", () => {
    expect(progressPercent(null)).toBeNull();
    expect(progressPercent(progress({ done: 3, total: 10 }))).toBe(30);
    expect(progressPercent(progress({ done: 12, total: 10 }))).toBe(100);
    expect(progressPercent(progress({ stage: "database", done: 0, total: 0 }))).toBeNull();
    expect(progressPercent(progress({ stage: "done", done: 0, total: 0 }))).toBe(100);
  });

  it("names the stage and the track being written", () => {
    expect(progressLine(null)).toBe("Starting…");
    expect(progressLine(progress())).toBe("Copying 3 of 10 · Signal Fire");
    expect(progressLine(progress({ stage: "native_stems", done: 1, total: 2, track: { id: null, title: "Kiln" } }))).toBe("Writing Native stems 1 of 2 · Kiln");
    expect(progressLine(progress({ stage: "database", done: 0, total: 0, track: null }))).toBe("Writing the Stick's database");
    expect(progressLine(progress({ stage: "verifying", done: 0, total: 0, track: null }))).toBe("Checking what was written");
  });
});
