import { describe, expect, it } from "vitest";

import { coalescedRefresh } from "./coalescedRefresh";

/** A read the test finishes by hand. */
function heldReads() {
  const waiting: (() => void)[] = [];
  let calls = 0;
  const run = () => {
    calls += 1;
    return new Promise<void>((resolve) => { waiting.push(resolve); });
  };
  const release = async () => {
    waiting.shift()?.();
    // Let the refresher's own continuation run.
    await Promise.resolve();
    await Promise.resolve();
  };
  return { run, release, calls: () => calls, waiting: () => waiting.length };
}

describe("coalescedRefresh", () => {
  it("keeps one read in flight and one pending, however many ask while it runs", async () => {
    const reads = heldReads();
    const refresh = coalescedRefresh(reads.run);

    for (let i = 0; i < 100; i += 1) refresh.request();
    expect(reads.calls()).toBe(1);

    await reads.release();
    // The burst became exactly one more read, which sees everything it asked about.
    expect(reads.calls()).toBe(2);
    await reads.release();
    expect(reads.calls()).toBe(2);
    expect(reads.waiting()).toBe(0);
  });

  it("reads again when asked after a read has finished", async () => {
    const reads = heldReads();
    const refresh = coalescedRefresh(reads.run);
    refresh.request();
    await reads.release();
    refresh.request();
    expect(reads.calls()).toBe(2);
  });

  it("goes on after a read that failed", async () => {
    let calls = 0;
    const refresh = coalescedRefresh(() => {
      calls += 1;
      return calls === 1 ? Promise.reject(new Error("not yet")) : Promise.resolve();
    });
    refresh.request();
    refresh.request();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toBe(2);
  });

  it("starts nothing once stopped, not even the pending read", async () => {
    const reads = heldReads();
    const refresh = coalescedRefresh(reads.run);
    refresh.request();
    refresh.request();
    refresh.stop();
    await reads.release();
    refresh.request();
    expect(reads.calls()).toBe(1);
  });
});
