import { beforeEach, describe, expect, it } from "vitest";

import { fromServiceLoad, type LoadOutcome } from "@/lib/stickLoad";
import { resetStickLoads, runStickLoad, stickIdentity, stickLoads } from "./stickLoads";

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
