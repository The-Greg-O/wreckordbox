import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { fromServiceLoad, type LoadOutcome } from "@/lib/stickLoad";
import {
  receiveStickLoadProgress, resetStickLoads, restoreStickLoad, runStickLoad, shrinkStickLoad, stickIdentity, stickLoads,
  subscribeStickLoads, takeWantedStick, showStickLoad,
} from "./stickLoads";

const A = { identity: stickIdentity("/media/A", "dev:1"), path: "/media/A", name: "A" };
const B = { identity: stickIdentity("/media/B", "dev:2"), path: "/media/B", name: "B" };
const outcome = (): LoadOutcome => fromServiceLoad({ stick: { name: "A" }, outcome: "loaded", written: {}, not_written: {}, unchanged: {}, native_stems: [] });

describe("stickLoads", () => {
  beforeEach(() => { resetStickLoads(); });

  it("keeps a load's report, and a failure, whatever happens to the view", async () => {
    await runStickLoad(A, async (run) => { await run(() => Promise.resolve(outcome())); return true; });
    await runStickLoad(B, async (run) => { await run(() => Promise.reject(new Error("removed during the export"))); return true; });
    expect(stickLoads().map((l) => [l.name, l.state, l.error])).toEqual([["A", "done", null], ["B", "failed", "removed during the export"]]);
  });

  it("a hold cancelled before the load ran leaves the Stick's earlier report", async () => {
    await runStickLoad(A, async (run) => { await run(() => Promise.resolve(outcome())); return true; });
    await runStickLoad(A, () => Promise.resolve(false));
    expect(stickLoads().map((l) => l.state)).toEqual(["done"]);
  });

  it("the newest report for a Stick replaces its older ones, not another Stick's", async () => {
    await runStickLoad(A, async (run) => { await run(() => Promise.resolve(outcome())); return true; });
    await runStickLoad(B, async (run) => { await run(() => Promise.resolve(outcome())); return true; });
    await runStickLoad(A, async (run) => { await run(() => Promise.reject(new Error("again"))); return true; });
    expect(stickLoads().map((l) => [l.name, l.state])).toEqual([["B", "done"], ["A", "failed"]]);
  });

  it("a Stick replaced at the same path is another Stick", () => {
    expect(stickIdentity("/media/A", "dev:1")).not.toBe(stickIdentity("/media/A", "dev:9"));
  });

  it("a refusal before the hold is reported, not lost", async () => {
    await runStickLoad(A, () => Promise.reject(new Error("no confirmation here")));
    expect(stickLoads().map((l) => [l.state, l.error])).toEqual([["failed", "no confirmation here"]]);
  });
});

/** A load that runs until `finish` is called. */
function running(stick = A) {
  let finish: (result: LoadOutcome | Error) => void = () => undefined;
  const done = runStickLoad(stick, async (run) => {
    await run(() => new Promise<LoadOutcome>((resolve, reject) => {
      finish = (result) => { if (result instanceof Error) reject(result); else resolve(result); };
    }));
    return true;
  });
  return { done, finish: (result: LoadOutcome | Error) => finish(result) };
}

const event = (stick: string, done: number, title = `Track ${done}`) =>
  ({ stick, stage: "copying", done, total: 500, track: { id: String(done), title } });

describe("a running load (wbx-72)", () => {
  beforeEach(() => { resetStickLoads(); vi.useFakeTimers(); });
  afterEach(() => { vi.useRealTimers(); });

  it("is loading, can be shrunk and brought back, and ends finished or failed by its report", async () => {
    const load = running();
    await vi.advanceTimersByTimeAsync(0);
    const id = stickLoads()[0]!.id;
    expect(stickLoads()[0]!.machine).toEqual({ state: "loading", shrunk: false, progress: null });
    shrinkStickLoad(id);
    expect(stickLoads()[0]!.machine).toMatchObject({ state: "loading", shrunk: true });
    restoreStickLoad(id);
    expect(stickLoads()[0]!.machine).toMatchObject({ state: "loading", shrunk: false });
    shrinkStickLoad(id);
    load.finish(outcome());
    await load.done;
    expect(stickLoads()[0]!.machine).toEqual({ state: "finished", outcome: outcome() });

    const again = running();
    await vi.advanceTimersByTimeAsync(0);
    again.finish(new Error("removed during the export"));
    await again.done;
    expect(stickLoads().at(-1)!.machine).toEqual({ state: "failed", outcome: null, error: "removed during the export" });
  });

  it("coalesces a burst of progress: one update, and it is the latest", async () => {
    const load = running();
    await vi.advanceTimersByTimeAsync(0);
    let told = 0;
    const stop = subscribeStickLoads(() => { told += 1; });
    for (let at = 1; at <= 500; at += 1) receiveStickLoadProgress(event("/media/A", at));
    expect(told).toBe(0);
    await vi.advanceTimersByTimeAsync(200);
    expect(told).toBe(1);
    expect(stickLoads()[0]!.machine).toMatchObject({ state: "loading", progress: { done: 500, track: { title: "Track 500" } } });
    stop();
    load.finish(outcome());
    await load.done;
  });

  it("progress for another Stick, or after the load ended, changes nothing", async () => {
    const load = running();
    await vi.advanceTimersByTimeAsync(0);
    receiveStickLoadProgress(event("/media/B", 3));
    await vi.advanceTimersByTimeAsync(200);
    expect(stickLoads()[0]!.machine).toEqual({ state: "loading", shrunk: false, progress: null });
    load.finish(outcome());
    await load.done;
    receiveStickLoadProgress(event("/media/A", 9));
    await vi.advanceTimersByTimeAsync(200);
    expect(stickLoads()[0]!.machine.state).toBe("finished");
  });

  it("showing a load from the status line brings it back and asks the Stick view for its Stick", async () => {
    const load = running();
    await vi.advanceTimersByTimeAsync(0);
    const id = stickLoads()[0]!.id;
    shrinkStickLoad(id);
    showStickLoad(id);
    expect(stickLoads()[0]!.machine).toMatchObject({ state: "loading", shrunk: false });
    expect(takeWantedStick()).toBe("/media/A");
    expect(takeWantedStick()).toBeNull();
    load.finish(outcome());
    await load.done;
  });
});
