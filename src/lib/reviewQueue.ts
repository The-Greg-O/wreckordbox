/**
 * The Review view's queue (wbx-74), in the view's terms.
 *
 * The Facts waiting for the DJ are the Library service's (`review.waiting`,
 * wbx-27): the view adds no logic beyond selection and the confidence
 * threshold. Every accept carries the token of the listing the view SHOWS,
 * so this module keeps what is shown apart from what was last read (the
 * "board"): while the DJ's pointer or focus is on the list, a new listing
 * is held and announced, never swapped in under them; and a listing shown
 * after the service refused a decision marks what changed so the DJ can
 * decide again.
 */
import type { RowDto } from "@/ipc/types";

/** The fields Projection writes (`projection::Field`). */
export type ReviewField = "energy" | "prep_colour" | (string & {});

export interface ReviewSource {
  kind: string;
  name: string;
}

/** One Fact waiting for the DJ (the service's `Held`). */
export interface WaitingFact {
  fact: number;
  track: string;
  title: string;
  field: ReviewField;
  /** What the Library holds now. */
  before: unknown;
  /** What accepting would make it. */
  after: unknown;
  source: ReviewSource;
  confidence: number;
  /** Why Projection held it back: `waits_for_dj`, `below_confidence`, or another the service names. */
  reason: string;
  threshold: number | undefined;
}

/** A Run some waiting Facts were recorded in. */
export interface ReviewRun {
  id: number;
  source: ReviewSource;
  description: string;
  startedAt: number;
  /** `null` while the Run is still working. */
  finishedAt: number | null;
  facts: number[];
}

/** The whole queue as one read gave it. */
export interface ReviewListing {
  facts: WaitingFact[];
  /** Names exactly this listing (the CLI's binding). */
  token: string;
  /** Each listed Fact's own token: what an accept of chosen rows sends (wbx-74 review). */
  rowTokens: Record<number, string>;
  runs: ReviewRun[];
  libraryProtected: boolean;
  /** The deck's row of each listed track, by track id. */
  decks: Record<string, RowDto>;
}

type Json = Record<string, unknown>;
const obj = (v: unknown): Json => (v && typeof v === "object" ? (v as Json) : {});
const arr = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);
const num = (v: unknown, or = 0): number => (typeof v === "number" && Number.isFinite(v) ? v : or);
const str = (v: unknown): string => (typeof v === "string" ? v : "");
const source = (v: unknown): ReviewSource => ({ kind: str(obj(v).kind), name: str(obj(v).name) });

/** The window's `review_waiting` answer: `{ listing: <review.waiting>, rows: RowDto[] }`. */
export function fromServiceListing(answer: unknown): ReviewListing {
  const outer = obj(answer);
  const listing = obj(outer.listing);
  const decks: Record<string, RowDto> = {};
  for (const row of arr(outer.rows)) {
    const r = row as RowDto;
    if (r && typeof r.id === "string") decks[r.id] = r;
  }
  return {
    facts: arr(listing.waiting).map((h) => {
      const w = obj(h);
      return {
        fact: num(w.fact),
        track: str(w.track),
        title: str(w.title),
        field: str(w.field),
        before: w.before ?? null,
        after: w.after ?? null,
        source: source(w.source),
        confidence: num(w.confidence),
        reason: str(w.reason),
        threshold: typeof w.threshold === "number" ? w.threshold : undefined,
      };
    }),
    token: str(listing.token),
    rowTokens: Object.fromEntries(Object.entries(obj(listing.row_tokens))
      .filter(([, t]) => typeof t === "string")
      .map(([fact, t]) => [Number(fact), t as string])),
    runs: arr(listing.runs).map((r) => {
      const run = obj(r);
      return {
        id: num(run.id),
        source: source(run.source),
        description: str(run.description),
        startedAt: num(run.started_at),
        finishedAt: typeof run.finished_at === "number" ? run.finished_at : null,
        facts: arr(run.facts).map((f) => num(f)),
      };
    }),
    libraryProtected: listing.library_protected === true,
    decks,
  };
}

/** A Run's waiting Facts; `run` is `null` for Facts whose Run was not named. */
export interface RunGroup {
  run: ReviewRun | null;
  facts: WaitingFact[];
}

/**
 * Grouped by Run, lowest Run first, each Run's Facts by identifier: a Fact
 * recorded later lands at the end of its Run, and a new Run after the
 * others, so arrivals add below what is shown rather than above it.
 */
export function groupByRun(listing: ReviewListing): RunGroup[] {
  const byFact = new Map(listing.facts.map((f) => [f.fact, f]));
  const placed = new Set<number>();
  const groups: RunGroup[] = [];
  for (const run of [...listing.runs].sort((a, b) => a.id - b.id)) {
    const facts = run.facts
      .map((id) => byFact.get(id))
      .filter((f): f is WaitingFact => f !== undefined && !placed.has(f.fact))
      .sort((a, b) => a.fact - b.fact);
    for (const f of facts) placed.add(f.fact);
    if (facts.length > 0) groups.push({ run, facts });
  }
  const rest = listing.facts.filter((f) => !placed.has(f.fact)).sort((a, b) => a.fact - b.fact);
  if (rest.length > 0) groups.push({ run: null, facts: rest });
  return groups;
}

/** The Facts at or above `threshold`, in the order given. */
export function atOrAbove(facts: readonly WaitingFact[], threshold: number): number[] {
  return facts.filter((f) => f.confidence >= threshold - 1e-9).map((f) => f.fact);
}

/** Below this an agent was unsure: drawn to stand out (docs/design/screens/review). */
export const LOW_CONFIDENCE = 0.6;

export function isLow(confidence: number): boolean {
  return confidence < LOW_CONFIDENCE;
}

/** A value as the row shows it. */
export function valueText(field: ReviewField, value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (field === "prep_colour" && typeof value === "object") {
    const v = value as { label?: unknown; colour?: unknown };
    return str(v.label) || str(v.colour) || "—";
  }
  return typeof value === "string" ? value : JSON.stringify(value);
}

/** What a field is called on the row. */
export function fieldText(field: ReviewField): string {
  if (field === "energy") return "Energy";
  if (field === "prep_colour") return "Prep colour";
  return field;
}

/** Why the Fact waits, for the DJ. */
export function reasonText(fact: Pick<WaitingFact, "reason" | "threshold">): string {
  if (fact.reason === "waits_for_dj") return "waits for you";
  if (fact.reason === "below_confidence") {
    return fact.threshold === undefined ? "held: below the confidence set" : `held: below ${fact.threshold.toFixed(2)}`;
  }
  return `held: ${fact.reason.replace(/_/g, " ")}`;
}

export function sourceText(s: ReviewSource): string {
  return s.name ? `${s.kind} ${s.name}` : s.kind;
}

/** Everything about a row that decides what accepting it would do. */
function rowKey(f: WaitingFact): string {
  return JSON.stringify([f.track, f.field, f.before, f.after, f.source, f.confidence, f.reason, f.threshold ?? null]);
}

export interface ListingChanges {
  added: number[];
  gone: number[];
  changed: number[];
}

export function listingChanges(shown: ReviewListing, latest: ReviewListing): ListingChanges {
  const before = new Map(shown.facts.map((f) => [f.fact, rowKey(f)]));
  const after = new Map(latest.facts.map((f) => [f.fact, rowKey(f)]));
  const ids = (m: Map<number, string>) => [...m.keys()].sort((a, b) => a - b);
  return {
    added: ids(after).filter((id) => !before.has(id)),
    gone: ids(before).filter((id) => !after.has(id)),
    changed: ids(after).filter((id) => before.has(id) && before.get(id) !== after.get(id)),
  };
}

function nothing(c: ListingChanges): boolean {
  return c.added.length === 0 && c.gone.length === 0 && c.changed.length === 0;
}

/** What is shown, what was last read, and what the DJ has chosen. */
export interface Board {
  shown: ReviewListing | null;
  latest: ReviewListing | null;
  /** The DJ is on the list (pointer or focus): nothing shown moves. */
  pinned: boolean;
  /** Chosen Facts, by identifier: always rows of `shown`. */
  selected: ReadonlySet<number>;
  /** After a refusal: what changed from the listing the DJ decided on. */
  marked: ListingChanges | null;
  /**
   * Rows the DJ just decided, kept in their place (inert, marked with what
   * was done) while the DJ is still on the list, so the row below never
   * slides under the pointer. Let go when the DJ leaves the list.
   */
  ghosts: ReadonlyMap<number, Ghost>;
  /**
   * After a refusal: what is shown stays until the DJ asks for the list as
   * it is now (`showLatest`); no decision is offered meanwhile, and no
   * read, pin or unpin replaces it.
   */
  held: boolean;
  /** Rows the DJ's decision applied, drawn inert until a read shows the result. */
  deciding: ReadonlyMap<number, string>;
}

/** The rows of `listing` named by `facts`, each with its own token: what an accept sends. */
export function rowsToAccept(listing: ReviewListing, facts: readonly number[]): { fact: number; token: string }[] {
  return facts.map((fact) => ({ fact, token: listing.rowTokens[fact] ?? "" }));
}

/** A decided row held in place. */
export interface Ghost {
  fact: WaitingFact;
  run: ReviewRun | null;
  verb: string;
}

export function emptyBoard(): Board {
  return { shown: null, latest: null, pinned: false, selected: new Set(), marked: null, ghosts: new Map(), held: false, deciding: new Map() };
}

function show(board: Board, listing: ReviewListing, marked: ListingChanges | null): Board {
  const keep = new Set<number>();
  if (board.shown) {
    const was = new Map(board.shown.facts.map((f) => [f.fact, rowKey(f)]));
    for (const f of listing.facts) {
      if (board.selected.has(f.fact) && was.get(f.fact) === rowKey(f)) keep.add(f.fact);
    }
  }
  return { ...board, shown: listing, latest: listing, selected: keep, marked, ghosts: board.pinned ? board.ghosts : new Map(), deciding: new Map() };
}

/**
 * A listing was read. Shown at once unless the DJ is on the list; with
 * `force` (the DJ's decision was refused because something changed) it is
 * shown anyway, with what changed marked.
 */
export function received(board: Board, listing: ReviewListing, options: { force?: boolean } = {}): Board {
  if (options.force) {
    return show({ ...board, ghosts: new Map() }, listing, board.shown ? listingChanges(board.shown, listing) : null);
  }
  if (board.held && board.shown !== null) return { ...board, latest: listing };
  if (board.shown === null || !board.pinned) return show(board, listing, board.pinned ? board.marked : null);
  return { ...board, latest: listing };
}

/** The DJ came onto the list, or left it; leaving shows what was held. */
export function pinBoard(board: Board, pinned: boolean): Board {
  if (board.pinned === pinned) return board;
  const next: Board = pinned ? { ...board, pinned } : { ...board, pinned, ghosts: new Map() };
  if (!pinned && !next.held && next.latest && next.latest !== next.shown) return show(next, next.latest, null);
  return next;
}

/**
 * The listing read after the DJ's own decision on `facts` (`verb`:
 * "accepted", "rejected"). Not on the list: shown. On it: shown with the
 * decided rows held in place when that decision is all that changed;
 * otherwise held, like any other change, until the DJ leaves the list.
 */
export function afterDecision(board: Board, listing: ReviewListing, facts: readonly number[], verb: string): Board {
  if (board.held) return { ...board, latest: listing };
  if (!board.shown || !board.pinned) return show(board, listing, null);
  const c = listingChanges(board.shown, listing);
  const ours = new Set(facts);
  if (c.added.length > 0 || c.changed.length > 0 || !c.gone.every((id) => ours.has(id))) {
    return { ...board, latest: listing };
  }
  const ghosts = new Map(board.ghosts);
  for (const id of c.gone) {
    const fact = board.shown.facts.find((f) => f.fact === id);
    if (fact) ghosts.set(id, { fact, run: board.shown.runs.find((r) => r.facts.includes(id)) ?? null, verb });
  }
  return show({ ...board, ghosts }, listing, null);
}

/** The DJ's decision on `facts` came back applied: drawn inert at once, before any read. */
export function decidedNow(board: Board, facts: readonly number[], verb: string): Board {
  if (facts.length === 0) return board;
  const deciding = new Map(board.deciding);
  const selected = new Set(board.selected);
  for (const fact of facts) {
    deciding.set(fact, verb);
    selected.delete(fact);
  }
  return { ...board, deciding, selected };
}

/** The DJ's decision was refused (or not answered): what is shown stays until the DJ asks for the list as it is now. */
export function refusedBoard(board: Board): Board {
  return { ...board, held: true };
}

/** A row as the view draws it: a waiting Fact, or a decided one held in place (`ghost` says what was done). */
export interface ShownRow {
  fact: WaitingFact;
  ghost: string | null;
}

export interface ShownGroup {
  run: ReviewRun | null;
  rows: ShownRow[];
}

/** The shown listing grouped by Run, with decided rows held in their places. */
export function shownGroups(board: Board): ShownGroup[] {
  if (!board.shown) return [];
  if (board.ghosts.size === 0) {
    return groupByRun(board.shown).map((g) => ({
      run: g.run, rows: g.facts.map((fact) => ({ fact, ghost: board.deciding.get(fact.fact) ?? null })),
    }));
  }
  const runs = new Map(board.shown.runs.map((r) => [r.id, { ...r, facts: [...r.facts] }]));
  const facts = [...board.shown.facts];
  for (const ghost of board.ghosts.values()) {
    facts.push(ghost.fact);
    if (ghost.run) {
      const run = runs.get(ghost.run.id) ?? { ...ghost.run, facts: [] };
      run.facts.push(ghost.fact.fact);
      runs.set(run.id, run);
    }
  }
  const verbs = new Map([...board.ghosts].map(([id, g]) => [id, g.verb]));
  for (const [id, verb] of board.deciding) verbs.set(id, verb);
  return groupByRun({ ...board.shown, facts, runs: [...runs.values()] }).map((g) => ({
    run: g.run,
    rows: g.facts.map((fact) => ({ fact, ghost: verbs.get(fact.fact) ?? null })),
  }));
}

/** What a held listing would change, or `null` when nothing is held. */
export function pendingChanges(board: Board): ListingChanges | null {
  if (!board.shown || !board.latest || board.latest === board.shown) return null;
  const changes = listingChanges(board.shown, board.latest);
  return nothing(changes) ? null : changes;
}

/** The DJ asked to see what was held: shown now, decided rows let go. */
export function showLatest(board: Board): Board {
  if (!board.latest) return board;
  const marked = board.shown ? listingChanges(board.shown, board.latest) : null;
  const shown = show({ ...board, ghosts: new Map(), pinned: false, held: false }, board.latest, marked && !nothing(marked) ? marked : null);
  return { ...shown, pinned: board.pinned };
}

export function toggleSelected(board: Board, fact: number): Board {
  if (!board.shown?.facts.some((f) => f.fact === fact)) return board;
  const selected = new Set(board.selected);
  if (selected.has(fact)) selected.delete(fact);
  else selected.add(fact);
  return { ...board, selected };
}

export function clearSelected(board: Board): Board {
  return board.selected.size === 0 ? board : { ...board, selected: new Set() };
}

/** What a decision came to. */
export type DecisionOutcome =
  | { kind: "done"; count: number; facts: number[]; rejection: number | null }
  | { kind: "refused"; code: string; message: string }
  | { kind: "partial"; message: string; applied: number[] | null; notApplied: number[] | null };

const ids = (v: unknown): number[] | null => (Array.isArray(v) ? v.map((n) => num(n)) : null);

export function outcomeOf(answer: unknown): DecisionOutcome {
  const a = obj(answer);
  if (a.ok === true) {
    const decided = arr(a.decided ?? a.reopened).map((d) => {
      const row = obj(d);
      return num(row.fact ?? obj(row.offered).fact);
    });
    const rejection = obj(a.rejection).entry;
    return { kind: "done", count: num(a.count, decided.length), facts: decided, rejection: typeof rejection === "number" ? rejection : null };
  }
  const error = obj(a.error);
  const code = str(error.code) || "failed";
  const message = str(error.message);
  if (code === "review_partial") {
    return { kind: "partial", message, applied: ids(error.applied), notApplied: ids(error.notApplied) };
  }
  return { kind: "refused", code, message };
}

/** A suggestion standing rejected (`review.rejected`). */
export interface RejectedFact {
  fact: number;
  track: string;
  field: ReviewField;
  value: unknown;
  source: ReviewSource;
  confidence: number;
  /** The Ledger entry of the rejection it is in. */
  entry: number;
}

export function fromServiceRejected(answer: unknown): RejectedFact[] {
  return arr(obj(answer).rejected).map((r) => {
    const row = obj(r);
    const offered = obj(row.offered ?? row);
    return {
      fact: num(offered.fact),
      track: str(offered.track),
      field: str(offered.field),
      value: offered.value ?? null,
      source: source(offered.source),
      confidence: num(offered.confidence),
      entry: num(obj(row.rejection).entry),
    };
  });
}
