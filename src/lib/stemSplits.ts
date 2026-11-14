/**
 * Splitting tracks' Stems (wbx-67): the window's own queue of separations,
 * owned by the shell, not by a view, so a split survives switching views
 * and tracks and is shown both on the large player's Stems control and in
 * the status line.
 *
 * - One split at a time, as the service allows: the next waits, "queued".
 * - Never twice for one track: a request for a track already queued or
 *   running is "already" and asks nothing.
 * - Another client's separation holding the engine (`separation_busy`)
 *   puts the split back in the queue and tries again later.
 * - A failure keeps its reason until the DJ dismisses it or asks again.
 * - Progress events arrive in bursts (the engine reports every few tens of
 *   milliseconds); listeners are told at most once per `coalesceMs`, with
 *   the latest value.
 *
 * Starting a split is not a Library edit (it writes the Sidecar, through
 * the service) and never waits on, or holds up, anything the Intake view
 * does: Mark Clean and next moves on while a split runs.
 */

export interface StemsProgress { trackId: string; stage: string; fraction: number }

export type SplitOutcome =
  | { ok: true; reused: boolean }
  | { ok: false; error: { code: string; message: string } };

export type SplitPhase = "queued" | "running" | "done" | "failed";

export interface Split {
  trackId: string;
  title: string;
  phase: SplitPhase;
  /** What a queued split waits for: a track's title, or another client. */
  behind: string | null;
  stage: string | null;
  /** 0 to 1, the engine's estimate from the clock. */
  fraction: number | null;
  reason: string | null;
}

export interface StemSplitsDeps {
  separate(id: string, again: boolean): Promise<SplitOutcome>;
  onProgress(listener: (event: StemsProgress) => void): () => void;
  retryAfterMs?: number;
  coalesceMs?: number;
}

export type RequestAnswer = "started" | "queued" | "already";

export interface StemSplits {
  request(id: string, title: string, options?: { again?: boolean }): RequestAnswer;
  dismiss(id: string): void;
  of(id: string): Split | null;
  /** Every split the window knows of, oldest request first. */
  all(): readonly Split[];
  subscribe(listener: () => void): () => void;
}

const BUSY = "separation_busy";
const ANOTHER = "another separation";

export function createStemSplits(deps: StemSplitsDeps): StemSplits {
  const retryAfter = deps.retryAfterMs ?? 15_000;
  const coalesce = deps.coalesceMs ?? 200;
  let splits: Split[] = [];
  const again = new Set<string>();
  const listeners = new Set<() => void>();
  let pending: ReturnType<typeof setTimeout> | null = null;
  let retry: ReturnType<typeof setTimeout> | null = null;
  let progressSubscription: (() => void) | null = null;

  const notify = () => {
    if (pending !== null) { clearTimeout(pending); pending = null; }
    for (const listener of listeners) listener();
  };
  const notifySoon = () => {
    if (pending === null) pending = setTimeout(() => { pending = null; notify(); }, coalesce);
  };
  const replace = (id: string, change: Partial<Split>) => {
    splits = splits.map((split) => split.trackId === id ? { ...split, ...change } : split);
  };
  const running = () => splits.find((split) => split.phase === "running") ?? null;

  const ensureProgress = () => {
    if (progressSubscription) return;
    progressSubscription = deps.onProgress((event) => {
      const split = splits.find((candidate) => candidate.trackId === event.trackId);
      if (split?.phase !== "running") return;
      if (split.stage === event.stage && split.fraction === event.fraction) return;
      replace(event.trackId, { stage: event.stage, fraction: event.fraction });
      notifySoon();
    });
  };

  const queuedBehind = () => {
    const now = running();
    for (const split of splits) {
      if (split.phase === "queued" && split.behind !== ANOTHER) replace(split.trackId, { behind: now?.title ?? null });
    }
  };

  const start = (split: Split) => {
    replace(split.trackId, { phase: "running", behind: null, stage: null, fraction: null, reason: null });
    queuedBehind();
    const id = split.trackId;
    const asked = again.has(id);
    again.delete(id);
    deps.separate(id, asked).then(
      (outcome) => finish(id, outcome, asked),
      (error: unknown) => finish(id, { ok: false, error: { code: "failed", message: error instanceof Error ? error.message : String(error) } }, asked),
    );
  };

  const finish = (id: string, outcome: SplitOutcome, asked: boolean) => {
    if (outcome.ok) replace(id, { phase: "done", stage: null, fraction: 1 });
    else if (outcome.error.code === BUSY) {
      // Another client is separating: back of the queue, try again later.
      if (asked) again.add(id);
      const split = splits.find((candidate) => candidate.trackId === id);
      splits = [...splits.filter((candidate) => candidate.trackId !== id), { ...split!, phase: "queued", behind: ANOTHER, stage: null, fraction: null }];
      if (retry === null) {
        retry = setTimeout(() => {
          retry = null;
          for (const waiting of splits) if (waiting.behind === ANOTHER) replace(waiting.trackId, { behind: null });
          pump();
          notify();
        }, retryAfter);
      }
    } else replace(id, { phase: "failed", stage: null, fraction: null, reason: outcome.error.message });
    pump();
    notify();
  };

  const pump = () => {
    if (running()) return;
    const next = splits.find((split) => split.phase === "queued" && split.behind !== ANOTHER);
    if (next) start(next);
  };

  return {
    request(id, title, options) {
      const known = splits.find((split) => split.trackId === id);
      if (known && (known.phase === "queued" || known.phase === "running")) return "already";
      ensureProgress();
      if (options?.again) again.add(id);
      const split: Split = { trackId: id, title, phase: "queued", behind: null, stage: null, fraction: null, reason: null };
      splits = [...splits.filter((candidate) => candidate.trackId !== id), split];
      const before = running();
      pump();
      queuedBehind();
      notify();
      return before ? "queued" : running()?.trackId === id ? "started" : "queued";
    },
    dismiss(id) {
      const known = splits.find((split) => split.trackId === id);
      if (!known || known.phase === "queued" || known.phase === "running") return;
      splits = splits.filter((split) => split.trackId !== id);
      notify();
    },
    of: (id) => splits.find((split) => split.trackId === id) ?? null,
    all: () => splits,
    subscribe(listener) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}

/** What a split is doing, in words for the control and the status line. */
export function splitLine(split: Split): string {
  switch (split.phase) {
    case "queued": return split.behind ? `Queued behind ${split.behind === ANOTHER ? ANOTHER : `“${split.behind}”`}` : "Queued";
    case "running": return split.fraction === null || split.stage === null
      ? "Splitting: starting"
      : `Splitting: ${split.stage.replace(/_/g, " ")} ${Math.round(split.fraction * 100)}%`;
    case "done": return "Split done";
    case "failed": return `Split failed: ${split.reason ?? "unknown reason"}`;
  }
}

/**
 * Whether a Run the service lists is the separation of a split the window
 * already shows (its own line carries the progress and any failure), so the
 * status line names one split once. The service describes it as
 * `Separate track <id> "<title>" into Stems`.
 */
export function isShownSplitRun(description: string, splits: readonly Split[]): boolean {
  return splits.some((split) => (split.phase === "running" || split.phase === "failed")
    && description.startsWith(`Separate track ${split.trackId} `) && description.endsWith(" into Stems"));
}
