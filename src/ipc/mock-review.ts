/**
 * The mock backend's Review queue (wbx-74), answered the way the window's
 * `review_*` commands answer from the Library service (wbx-27):
 * `review_waiting` gives the waiting Facts with a token naming exactly that
 * listing and the Runs they were recorded in; an accept must carry the
 * token of the listing the view shows, and is refused `review_changed`
 * when anything listed changed since; every decision declares the DJ.
 *
 * Fixture: three Runs over the mock's first tracks. Run 41 "tagging pass"
 * (agent tagger) is still working; Run 38 "Beatport lookup" (agent web) is
 * finished and its Facts were held back below its confidence; Run 40 (tool
 * meter) proposes Prep colours. Some confidences are below 0.6.
 *
 * URL flags: `?review=empty` (nothing waiting), `?reviewProtected` (Library
 * Protection: every accept refused `library_protected`).
 *
 * Hooks on `window`: `__reviewArrive(n)` records n more Facts in the
 * working Run (a "row arriving live"); `__reviewChange(fact, after)`
 * changes a listed Fact's proposal; `__reviewFinishRun()` ends the working
 * Run; `__reviewCalls()` lists every decision asked (command, facts,
 * token, source); `__reviewNext(code)` answers the next accept with that
 * refusal (`"review_partial"` applies the first chosen Fact only);
 * `__reviewReads()` counts reads; `__holdReviewReads()` /
 * `__releaseReviewReads()`; `__failReviewReads(on)`; `__reviewOtherReject(fact)`
 * rejects as another client would (told, no Library change).
 */
import type { RowDto } from "./types";
import {
  fromServiceListing, fromServiceRejected, outcomeOf,
  type DecisionOutcome, type RejectedFact, type ReviewListing,
} from "@/lib/reviewQueue";

export interface MockReviewSource {
  all: readonly RowDto[];
  wait: <T>(value: T) => Promise<T>;
  ready: () => boolean;
  /** The mock's own Energy write (the edit path; Undo, `library:changed`). */
  rate: (id: string, stars: number) => Promise<unknown>;
  /** The mock's own Prep colour write. */
  colour: (id: string, colour: number) => Promise<unknown>;
}

interface Held {
  fact: number;
  track: string;
  field: "energy" | "prep_colour";
  before: unknown;
  after: unknown;
  source: { kind: string; name: string };
  confidence: number;
  reason: string;
  threshold?: number;
  run: number;
}

interface Run {
  id: number;
  source: { kind: string; name: string };
  description: string;
  started_at: number;
  finished_at: number | null;
}

interface Call {
  command: string;
  facts: number[];
  token?: string;
  rejection?: number | null;
  source: { kind: string };
}

const COLOURS: Record<number, { colour: string; label: string }> = {
  2: { colour: "Red", label: "Problem" },
  5: { colour: "Green", label: "Clean" },
  6: { colour: "Aqua", label: "" },
};

function readUrl(): URLSearchParams {
  return typeof location === "undefined" ? new URLSearchParams() : new URLSearchParams(location.search);
}

export function createMockReview(source: MockReviewSource) {
  const url = readUrl();
  const now = Date.now();
  const runs: Run[] = [
    { id: 38, source: { kind: "agent", name: "web" }, description: "Beatport lookup", started_at: now - 5 * 86_400_000, finished_at: now - 5 * 86_400_000 + 600_000 },
    { id: 40, source: { kind: "tool", name: "meter" }, description: "Prep colour from analysis", started_at: now - 86_400_000, finished_at: now - 86_400_000 + 60_000 },
    { id: 41, source: { kind: "agent", name: "tagger" }, description: "tagging pass", started_at: now - 3_600_000, finished_at: null },
  ];
  let waiting: Held[] = [];
  const rejected: { held: Held; entry: number }[] = [];
  let nextFact = 100;
  let nextEntry = 500;
  let version = 1;
  const calls: Call[] = [];
  let refuseNext: string | null = null;
  const reads = { waiting: 0 };
  let holding = false;
  let failing = false;
  let held: (() => void)[] = [];
  const listeners = new Set<() => void>();

  const track = (i: number) => source.all[i % Math.max(1, source.all.length)]!;
  const energy = (row: RowDto) => (row.rating ? row.rating : null);
  const add = (h: Omit<Held, "fact" | "before">) => {
    const row = source.all.find((r) => r.id === h.track);
    const before = h.field === "energy" ? (row ? energy(row) : null) : null;
    waiting.push({ ...h, fact: nextFact++, before });
  };
  if (url.get("review") !== "empty") {
    const confidences = [0.91, 0.62, 0.74, 0.83, 0.58, 0.88, 0.81, 0.44, 0.97];
    confidences.forEach((confidence, i) => {
      add({ track: track(i).id, field: "energy", after: 1 + (i % 5), source: runs[2]!.source, confidence, reason: "waits_for_dj", run: 41 });
    });
    [0.65, 0.52, 0.61].forEach((confidence, i) => {
      add({ track: track(10 + i).id, field: "energy", after: 2 + i, source: runs[0]!.source, confidence, reason: "below_confidence", threshold: 0.7, run: 38 });
    });
    [5, 2].forEach((colour, i) => {
      add({ track: track(14 + i).id, field: "prep_colour", after: { id: colour, ...COLOURS[colour] }, source: runs[1]!.source, confidence: 0.8 + i * 0.1, reason: "waits_for_dj", run: 40 });
    });
  }

  const changed = () => {
    version += 1;
    for (const listener of listeners) listener();
  };
  const token = () => `mock.${version}`;

  const answer = () => {
    const titled = waiting.map((h) => ({
      track: h.track, title: source.all.find((r) => r.id === h.track)?.title ?? h.track, field: h.field,
      before: h.before, after: h.after, fact: h.fact, source: h.source, confidence: h.confidence, reason: h.reason,
      ...(h.threshold === undefined ? {} : { threshold: h.threshold }),
    }));
    const used = runs.filter((r) => waiting.some((h) => h.run === r.id));
    const tracks = [...new Set(waiting.map((h) => h.track))];
    return {
      listing: {
        waiting: titled, count: titled.length, library_protected: url.has("reviewProtected"), token: token(),
        runs: used.map((r) => ({ ...r, outcome: null, facts: waiting.filter((h) => h.run === r.id).map((h) => h.fact) })),
      },
      rows: tracks.map((id) => source.all.find((r) => r.id === id)).filter((r): r is RowDto => r !== undefined),
    };
  };

  const refuse = (code: string, message: string) => ({ ok: false, error: { code, message } });

  const accept = async (facts: number[], given: string): Promise<DecisionOutcome> => {
    calls.push({ command: "review.accept", facts: [...facts], token: given, source: { kind: "dj" } });
    await source.wait(null);
    if (given !== token()) {
      return outcomeOf(refuse("review_changed", "what you were shown has changed: list the waiting Facts again and decide on what is listed now"));
    }
    const chosen = facts.map((f) => waiting.find((h) => h.fact === f));
    if (chosen.some((h) => h === undefined)) return outcomeOf(refuse("review_not_waiting", "a Fact named is not waiting: nothing was done"));
    if (url.has("reviewProtected")) return outcomeOf(refuse("library_protected", "Library Protection is on: nothing was written."));
    const refusal = refuseNext;
    refuseNext = null;
    if (refusal && refusal !== "review_partial") return outcomeOf(refuse(refusal, `refused: ${refusal}`));
    const apply = refusal === "review_partial" ? chosen.slice(0, 1) : chosen;
    for (const h of apply as Held[]) {
      if (h.field === "energy") await source.rate(h.track, Number(h.after));
      else await source.colour(h.track, Number((h.after as { id: number }).id));
    }
    const applied = new Set((apply as Held[]).map((h) => h.fact));
    waiting = waiting.filter((h) => !applied.has(h.fact));
    changed();
    if (refusal === "review_partial") {
      const not = facts.filter((f) => !applied.has(f));
      return outcomeOf({
        ok: false,
        error: {
          code: "review_partial",
          message: `the accept stopped part-way (the Library refused a write); applied: [${[...applied].join(", ")}], recorded as the DJ's and undoable in one step; not applied: [${not.join(", ")}], still waiting`,
          applied: [...applied], notApplied: not,
        },
      });
    }
    return outcomeOf({ ok: true, count: apply.length, decided: (apply as Held[]).map((h) => ({ fact: h.fact })) });
  };

  const reject = async (facts: number[], by = "dj"): Promise<DecisionOutcome> => {
    if (by === "dj") calls.push({ command: "review.reject", facts: [...facts], source: { kind: "dj" } });
    await source.wait(null);
    const chosen = facts.map((f) => waiting.find((h) => h.fact === f));
    if (chosen.some((h) => h === undefined)) return outcomeOf(refuse("review_not_waiting", "a Fact named is not waiting: nothing was done"));
    const entry = nextEntry++;
    for (const h of chosen as Held[]) rejected.push({ held: h, entry });
    const gone = new Set(facts);
    waiting = waiting.filter((h) => !gone.has(h.fact));
    changed();
    return outcomeOf({ ok: true, count: facts.length, decided: facts.map((fact) => ({ fact })), rejection: { entry, run: 900 + entry, at: Date.now() } });
  };

  const reopen = async (facts: number[], rejection: number | null): Promise<DecisionOutcome> => {
    calls.push({ command: "review.reopen", facts: [...facts], rejection, source: { kind: "dj" } });
    await source.wait(null);
    const chosen = rejection !== null
      ? rejected.filter((r) => r.entry === rejection)
      : rejected.filter((r) => facts.includes(r.held.fact));
    if (rejection === null && chosen.length < facts.length) {
      return outcomeOf(refuse("review_not_waiting", "a Fact named is not standing rejected: nothing was reopened"));
    }
    for (const r of chosen) {
      rejected.splice(rejected.indexOf(r), 1);
      waiting.push(r.held);
    }
    changed();
    return outcomeOf({ ok: true, count: chosen.length, reopened: chosen.map((r) => ({ fact: r.held.fact })) });
  };

  if (typeof window !== "undefined") {
    const hooks = window as unknown as Record<string, unknown>;
    hooks.__reviewArrive = (n: number) => {
      for (let i = 0; i < n; i += 1) {
        const k = nextFact;
        add({ track: track(20 + (k % 30)).id, field: "energy", after: 1 + (k % 5), source: runs[2]!.source, confidence: 0.5 + ((k * 7) % 50) / 100, reason: "waits_for_dj", run: 41 });
      }
      changed();
    };
    hooks.__reviewChange = (fact: number, after: unknown) => {
      const h = waiting.find((w) => w.fact === fact);
      if (h) h.after = after;
      changed();
    };
    hooks.__reviewFinishRun = () => {
      runs[2]!.finished_at = Date.now();
      changed();
    };
    hooks.__reviewCalls = () => calls.map((c) => ({ ...c }));
    hooks.__reviewNext = (code: string | null) => { refuseNext = code; };
    hooks.__reviewReads = () => ({ ...reads });
    hooks.__holdReviewReads = () => { holding = true; };
    hooks.__releaseReviewReads = () => {
      holding = false;
      const release = held;
      held = [];
      for (const r of release) r();
    };
    hooks.__failReviewReads = (on: boolean) => { failing = on; };
    hooks.__reviewOtherReject = (fact: number) => reject([fact], "other");
    hooks.__reviewWaiting = () => waiting.map((h) => h.fact);
  }

  return {
    reviewWaiting: async (): Promise<ReviewListing> => {
      reads.waiting += 1;
      if (holding) await new Promise<void>((resolve) => held.push(resolve));
      if (!source.ready()) throw new Error("The Library has not finished loading.");
      if (failing) throw new Error("The Facts waiting for you could not be read: the Sidecar is being written by another program");
      return fromServiceListing(await source.wait(answer()));
    },
    reviewAccept: accept,
    reviewReject: (facts: number[]) => reject(facts),
    reviewRejected: async (): Promise<RejectedFact[]> =>
      fromServiceRejected(await source.wait({
        rejected: rejected.map((r) => ({
          fact: r.held.fact, track: r.held.track, field: r.held.field, value: r.held.after,
          source: r.held.source, confidence: r.held.confidence, rejection: { entry: r.entry, run: 900 + r.entry, at: 0 },
        })),
      })),
    reviewReopen: reopen,
    onReviewChanged: (listener: () => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}
