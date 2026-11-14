/**
 * Stems in the mock backend (wbx-67): a fake separation, never a real one.
 * It reports progress in bursts as the engine does (every 50 ms), keeps a
 * Run open while it works (shown in the status line), records Stems, and
 * sends `sidecar:changed` with kind `stems`. One at a time, as the service:
 * a second request for the same track joins the first; another track is
 * answered `separation_busy`.
 *
 * URL: `?stems=<id>:current,<id>:stale` seeds Stems; `?stemsDelay=<ms>`
 * (default 1500) is how long a split takes.
 *
 * Hooks: `__stemsRuns(id?)` (separations the fake engine was asked for),
 * `__stemsReads()` (status reads), `__holdStems()` / `__releaseStems()`
 * (a split finishes only once released), `__failStems(message | null)`
 * (the next splits fail with this reason), `__stemsBurst(n)` (n progress
 * events at once for the running split), `__setStems(id, state)` (Stems
 * changed by another client, announced), `__otherClientSplitting(id | null)`
 * (another client's separation holds the engine), `__failStemsReads(on)`.
 */
import type { ActiveRun } from "./serviceActivity";
import type { SplitOutcome, StemsProgress } from "@/lib/stemSplits";
import type { StemsStatus, StemsState } from "@/lib/stemsStatus";

interface Job {
  id: string;
  fraction: number;
  timer: ReturnType<typeof setTimeout> | undefined;
  waiters: ((outcome: SplitOutcome) => void)[];
  runId: number;
}

export function createMockStems(deps: {
  titleOf: (id: string) => string;
  hasFile: (id: string) => boolean;
  ready: () => boolean;
}) {
  const states = new Map<string, { state: StemsState; madeAt: number }>();
  const progressListeners = new Set<(event: StemsProgress) => void>();
  const sidecarListeners = new Set<(change: { trackId: string; kind: string }) => void>();
  const runs = new Map<string, number>();
  let reads = 0;
  let job: Job | null = null;
  let held = false;
  let heldFinish: (() => void) | null = null;
  let failWith: string | null = null;
  let failReads = false;
  let other: string | null = null;
  let nextRun = 900;
  const params = typeof window === "undefined" ? new URLSearchParams() : new URLSearchParams(window.location.search);
  const delay = Number(params.get("stemsDelay") ?? 1500);
  for (const seed of (params.get("stems") ?? "").split(",").filter(Boolean)) {
    const [id, state] = seed.split(":");
    if (id && (state === "current" || state === "stale")) states.set(id, { state, madeAt: Date.now() - 3_600_000 });
  }

  const announce = (id: string) => { for (const listener of sidecarListeners) listener({ trackId: id, kind: "stems" }); };
  const tell = (event: StemsProgress) => { for (const listener of progressListeners) listener(event); };

  const finish = (current: Job) => {
    clearTimeout(current.timer);
    const outcome: SplitOutcome = failWith !== null
      ? { ok: false, error: { code: "separation_failed", message: failWith } }
      : { ok: true, reused: false };
    if (outcome.ok) {
      states.set(current.id, { state: "current", madeAt: Date.now() });
      announce(current.id);
    }
    job = null;
    for (const waiter of current.waiters) waiter(outcome);
  };

  const stemsStatus = (id: string): Promise<StemsStatus> => {
    reads += 1;
    if (!deps.ready()) return Promise.reject(new Error("The Library has not finished loading."));
    if (failReads) return Promise.reject(new Error("the Sidecar could not be read"));
    const known = states.get(id);
    return Promise.resolve({
      state: known?.state ?? "none",
      madeAt: known?.madeAt ?? null,
      separating: job?.id ?? other,
      unavailable: deps.hasFile(id) ? null : { code: "track_file_missing", message: `the audio file of track ${id} is missing` },
    });
  };

  const stemsSeparate = (id: string, again: boolean): Promise<SplitOutcome> => {
    if (job && job.id === id) return new Promise((resolve) => job!.waiters.push(resolve));
    if (job || other) {
      return Promise.resolve({ ok: false, error: { code: "separation_busy", message: `track ${job?.id ?? other} is being separated; one track at a time. Try again when it is done.` } });
    }
    if (!deps.hasFile(id)) return Promise.resolve({ ok: false, error: { code: "track_file_missing", message: `the audio file of track ${id} is missing` } });
    if (!again && states.get(id)?.state === "current") return Promise.resolve({ ok: true, reused: true });
    runs.set(id, (runs.get(id) ?? 0) + 1);
    return new Promise((resolve) => {
      const started = Date.now();
      const current: Job = { id, fraction: 0, waiters: [resolve], runId: nextRun++, timer: undefined };
      // The engine's reports, every 50 ms, until the split is done.
      const report = () => {
        const elapsed = Date.now() - started;
        current.fraction = Math.min(0.9, 0.05 + (elapsed / delay) * 0.85);
        tell({ trackId: id, stage: elapsed < delay * 0.1 ? "loading_model" : current.fraction >= 0.9 ? "writing" : "separating", fraction: current.fraction });
        if (elapsed < delay) current.timer = setTimeout(report, 50);
        else if (held) heldFinish = () => finish(current);
        else finish(current);
      };
      current.timer = setTimeout(report, 50);
      job = current;
    });
  };

  if (typeof window !== "undefined") {
    const hooks = window as unknown as Record<string, unknown>;
    hooks.__stemsRuns = (id?: string) => id === undefined ? [...runs.values()].reduce((a, b) => a + b, 0) : runs.get(id) ?? 0;
    hooks.__stemsReads = () => reads;
    hooks.__holdStems = () => { held = true; };
    hooks.__releaseStems = () => { held = false; const done = heldFinish; heldFinish = null; done?.(); };
    hooks.__failStems = (message: string | null) => { failWith = message; };
    hooks.__failStemsReads = (on: boolean) => { failReads = on; };
    // Each event in a task of its own, as IPC events arrive; resolves when all are sent.
    hooks.__stemsBurst = (n: number) => new Promise<void>((resolve) => {
      let sent = 0;
      const next = () => {
        if (!job || sent >= n) { resolve(); return; }
        sent += 1;
        // From 10% up, a different percent nearly every event.
        job.fraction = 0.1 + (sent - 1) * 0.007;
        tell({ trackId: job.id, stage: "separating", fraction: job.fraction });
        setTimeout(next, 1);
      };
      next();
    });
    hooks.__setStems = (id: string, state: StemsState | "none") => {
      if (state === "none") states.delete(id); else states.set(id, { state, madeAt: Date.now() });
      announce(id);
    };
    hooks.__otherClientSplitting = (id: string | null) => { other = id; };
  }

  return {
    stemsStatus,
    stemsSeparate,
    onStemsProgress: (listener: (event: StemsProgress) => void) => {
      progressListeners.add(listener);
      return () => { progressListeners.delete(listener); };
    },
    onSidecarChanged: (listener: (change: { trackId: string; kind: string }) => void) => {
      sidecarListeners.add(listener);
      return () => { sidecarListeners.delete(listener); };
    },
    /** The open Run of the split in progress, as `runs.active` lists it. */
    activeRuns: (): ActiveRun[] => job ? [{ id: job.runId, description: `Separate track ${job.id} "${deps.titleOf(job.id)}" into Stems` }] : [],
  };
}
