import { describe, expect, it, vi } from "vitest";

import { createStemSplits, isShownSplitRun, splitLine, type SplitOutcome, type StemsProgress } from "./stemSplits";

/** A separation the test answers by hand, and the progress it reports. */
function rig() {
  const asked: { id: string; again: boolean; answer: (outcome: SplitOutcome) => void }[] = [];
  let progress: (event: StemsProgress) => void = () => {};
  const notified = vi.fn();
  const splits = createStemSplits({
    separate: (id, again) => new Promise<SplitOutcome>((resolve) => { asked.push({ id, again, answer: resolve }); }),
    onProgress: (listener) => { progress = listener; return () => {}; },
    retryAfterMs: 1000,
    coalesceMs: 100,
  });
  splits.subscribe(notified);
  return { splits, asked, progress: (event: StemsProgress) => progress(event), notified };
}

const ok: SplitOutcome = { ok: true, reused: false };
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("stem splits", () => {
  it("runs one split at a time and queues the next", async () => {
    const { splits, asked } = rig();
    expect(splits.request("a", "Alpha")).toBe("started");
    expect(splits.request("b", "Beta")).toBe("queued");
    expect(asked.map((job) => job.id)).toEqual(["a"]);
    expect(splits.of("a")?.phase).toBe("running");
    expect(splits.of("b")).toMatchObject({ phase: "queued", behind: "Alpha" });

    asked[0]!.answer(ok);
    await flush();
    expect(splits.of("a")?.phase).toBe("done");
    expect(asked.map((job) => job.id)).toEqual(["a", "b"]);
    expect(splits.of("b")?.phase).toBe("running");
  });

  it("never asks twice for one track while it is queued or running", () => {
    const { splits, asked } = rig();
    splits.request("a", "Alpha");
    splits.request("b", "Beta");
    expect(splits.request("a", "Alpha")).toBe("already");
    expect(splits.request("b", "Beta")).toBe("already");
    expect(asked).toHaveLength(1);
  });

  it("keeps a failure with its reason until dismissed or asked again", async () => {
    const { splits, asked } = rig();
    splits.request("a", "Alpha");
    asked[0]!.answer({ ok: false, error: { code: "separation_failed", message: "the engine stopped" } });
    await flush();
    expect(splits.of("a")).toMatchObject({ phase: "failed", reason: "the engine stopped" });
    expect(splitLine(splits.of("a")!)).toBe("Split failed: the engine stopped");

    expect(splits.request("a", "Alpha")).toBe("started");
    expect(asked).toHaveLength(2);
    asked[1]!.answer({ ok: false, error: { code: "x", message: "again" } });
    await flush();
    splits.dismiss("a");
    expect(splits.of("a")).toBeNull();
  });

  it("a call that throws is a failure too, never a split left running", async () => {
    const splits = createStemSplits({
      separate: () => Promise.reject(new Error("no service")),
      onProgress: () => () => {},
    });
    splits.request("a", "Alpha");
    await flush();
    expect(splits.of("a")).toMatchObject({ phase: "failed", reason: "no service" });
  });

  it("waits and retries when another client's separation holds the engine", async () => {
    vi.useFakeTimers();
    try {
      const { splits, asked } = rig();
      splits.request("a", "Alpha");
      asked[0]!.answer({ ok: false, error: { code: "separation_busy", message: "track 7 is being separated" } });
      await vi.advanceTimersByTimeAsync(0);
      expect(splits.of("a")).toMatchObject({ phase: "queued", behind: "another separation" });
      expect(asked).toHaveLength(1);
      await vi.advanceTimersByTimeAsync(1000);
      expect(asked).toHaveLength(2);
      expect(splits.of("a")?.phase).toBe("running");
    } finally {
      vi.useRealTimers();
    }
  });

  it("coalesces a burst of progress into one notice and keeps the latest", async () => {
    vi.useFakeTimers();
    try {
      const { splits, progress, notified } = rig();
      splits.request("a", "Alpha");
      notified.mockClear();
      for (let i = 1; i <= 100; i++) progress({ trackId: "a", stage: "separating", fraction: i / 200 });
      progress({ trackId: "other", stage: "separating", fraction: 0.9 });
      expect(notified).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(100);
      expect(notified).toHaveBeenCalledTimes(1);
      expect(splits.of("a")).toMatchObject({ stage: "separating", fraction: 0.5 });
      expect(splitLine(splits.of("a")!)).toBe("Splitting: separating 50%");
    } finally {
      vi.useRealTimers();
    }
  });

  it("passes again through only when asked", () => {
    const { splits, asked } = rig();
    splits.request("a", "Alpha", { again: true });
    expect(asked[0]).toMatchObject({ id: "a", again: true });
  });

  it("describes each phase", () => {
    expect(splitLine({ trackId: "a", title: "A", phase: "queued", behind: "Beta", stage: null, fraction: null, reason: null })).toBe("Queued behind “Beta”");
    expect(splitLine({ trackId: "a", title: "A", phase: "running", behind: null, stage: null, fraction: null, reason: null })).toBe("Splitting: starting");
    expect(splitLine({ trackId: "a", title: "A", phase: "done", behind: null, stage: null, fraction: null, reason: null })).toBe("Split done");
  });
});

describe("isShownSplitRun", () => {
  const split = (trackId: string, phase: "running" | "queued" | "done" | "failed") =>
    ({ trackId, title: "T", phase, behind: null, stage: null, fraction: null, reason: null });
  it("is the Run of a split the window shows running or failed, and no other", () => {
    const run = 'Separate track 10 "T" into Stems';
    expect(isShownSplitRun(run, [split("10", "running")])).toBe(true);
    expect(isShownSplitRun(run, [split("10", "failed")])).toBe(true);
    expect(isShownSplitRun(run, [split("1", "running")])).toBe(false);
    expect(isShownSplitRun(run, [split("10", "done")])).toBe(false);
    expect(isShownSplitRun("Tag 12 tracks", [split("10", "running")])).toBe(false);
  });
});
