import { describe, expect, it } from "vitest";
import {
  atOrAbove,
  emptyBoard,
  fromServiceListing,
  groupByRun,
  isLow,
  listingChanges,
  outcomeOf,
  pendingChanges,
  pinBoard,
  received,
  toggleSelected,
  afterDecision,
  shownGroups,
  valueText,
  reasonText,
  type ReviewListing,
} from "./reviewQueue";

const held = (fact: number, track: string, confidence: number, after: unknown = 3, field = "energy") => ({
  track, title: `Track ${track}`, field, before: null, after, fact,
  source: { kind: "agent", name: "tagger" }, confidence, reason: "waits_for_dj",
});

function service(rows: ReturnType<typeof held>[], runs: { id: number; facts: number[]; finished?: boolean }[], token = "t1") {
  return {
    listing: {
      waiting: rows, count: rows.length, library_protected: false, token,
      runs: runs.map((r) => ({
        id: r.id, source: { kind: "agent", name: "tagger" }, description: `Run ${r.id}`,
        started_at: 1_700_000_000_000, finished_at: r.finished ? 1_700_000_100_000 : null, outcome: null, facts: r.facts,
      })),
    },
    rows: rows.map((r) => ({ id: r.track, title: r.title })),
  };
}

const listing = (rows: ReturnType<typeof held>[], runs: { id: number; facts: number[]; finished?: boolean }[], token = "t1"): ReviewListing =>
  fromServiceListing(service(rows, runs, token));

describe("the listing in the view's terms", () => {
  it("lifts the service's answer: facts, token, Runs and each track's deck row", () => {
    const l = listing([held(5, "10001", 0.9), held(7, "10002", 0.4)], [{ id: 2, facts: [5, 7] }]);
    expect(l.token).toBe("t1");
    expect(l.facts.map((f) => f.fact)).toEqual([5, 7]);
    expect(l.facts[0]).toMatchObject({ track: "10001", title: "Track 10001", field: "energy", before: null, after: 3, confidence: 0.9, source: { kind: "agent", name: "tagger" } });
    expect(l.runs[0]).toMatchObject({ id: 2, description: "Run 2", finishedAt: null, facts: [5, 7] });
    expect(l.decks["10002"]?.title).toBe("Track 10002");
  });

  it("groups by Run, lowest Run first, Facts by identifier, so a Fact that arrives later goes to the end of its Run", () => {
    const l = listing(
      [held(9, "10001", 0.9), held(3, "10002", 0.8), held(4, "10003", 0.7)],
      [{ id: 8, facts: [9] }, { id: 2, facts: [3, 4] }],
    );
    const groups = groupByRun(l);
    expect(groups.map((g) => g.run?.id)).toEqual([2, 8]);
    expect(groups[0]!.facts.map((f) => f.fact)).toEqual([3, 4]);
  });

  it("puts a Fact whose Run is not named in a group of its own rather than drop it", () => {
    const l = listing([held(3, "10002", 0.8), held(4, "10003", 0.7)], [{ id: 2, facts: [3] }]);
    const groups = groupByRun(l);
    expect(groups.map((g) => g.run?.id ?? null)).toEqual([2, null]);
    expect(groups[1]!.facts.map((f) => f.fact)).toEqual([4]);
  });

  it("chooses every Fact at or above a confidence, and only those", () => {
    const facts = listing([held(1, "a", 0.8), held(2, "b", 0.79), held(3, "c", 0.95)], [{ id: 1, facts: [1, 2, 3] }]).facts;
    expect(atOrAbove(facts, 0.8)).toEqual([1, 3]);
    expect(atOrAbove(facts, 0)).toEqual([1, 2, 3]);
    expect(atOrAbove(facts, 1)).toEqual([]);
  });

  it("calls a confidence below 0.6 low", () => {
    expect(isLow(0.59)).toBe(true);
    expect(isLow(0.6)).toBe(false);
  });

  it("words values and why a Fact was held back", () => {
    expect(valueText("energy", null)).toBe("—");
    expect(valueText("energy", 4)).toBe("4");
    expect(valueText("prep_colour", { id: 5, colour: "Green", label: "Clean" })).toBe("Clean");
    expect(valueText("prep_colour", { id: 5, colour: "Green", label: "" })).toBe("Green");
    expect(reasonText({ reason: "waits_for_dj", threshold: undefined })).toBe("waits for you");
    expect(reasonText({ reason: "below_confidence", threshold: 0.7 })).toBe("held: below 0.70");
    expect(reasonText({ reason: "duration mismatch", threshold: undefined })).toBe("held: duration mismatch");
  });
});

describe("what changed between two listings", () => {
  it("names new Facts, Facts gone and Facts whose row changed", () => {
    const before = listing([held(1, "a", 0.8), held(2, "b", 0.7)], [{ id: 1, facts: [1, 2] }]);
    const after = listing([held(1, "z", 0.8), held(3, "c", 0.7)], [{ id: 1, facts: [1, 3] }], "t2");
    expect(listingChanges(before, after)).toEqual({ added: [3], gone: [2], changed: [1] });
  });
});

describe("the board: rows arriving live never move under the pointer", () => {
  const first = listing([held(1, "a", 0.9), held(2, "b", 0.5)], [{ id: 1, facts: [1, 2] }], "t1");
  const more = listing([held(1, "a", 0.9), held(2, "b", 0.5), held(3, "c", 0.95)], [{ id: 1, facts: [1, 2, 3] }], "t2");

  it("shows a new listing at once while the DJ is not over the list", () => {
    const board = received(received(emptyBoard(), first), more);
    expect(board.shown?.token).toBe("t2");
    expect(pendingChanges(board)).toBeNull();
  });

  it("holds a new listing while pinned, says what is pending, and shows it when let go", () => {
    let board = pinBoard(received(emptyBoard(), first), true);
    board = received(board, more);
    expect(board.shown?.token).toBe("t1");
    expect(pendingChanges(board)).toEqual({ added: [3], gone: [], changed: [] });
    board = pinBoard(board, false);
    expect(board.shown?.token).toBe("t2");
    expect(pendingChanges(board)).toBeNull();
  });

  it("keeps a selection to rows still shown unchanged when a listing is shown", () => {
    let board = received(emptyBoard(), first);
    board = toggleSelected(toggleSelected(board, 1), 2);
    expect([...board.selected]).toEqual([1, 2]);
    const changed = listing([held(1, "a", 0.9), held(2, "b", 0.6)], [{ id: 1, facts: [1, 2] }], "t3");
    board = received(board, changed);
    // Fact 2's row changed: it is no longer chosen; the DJ chooses it again.
    expect([...board.selected]).toEqual([1]);
  });

  it("a forced listing (after a refusal) is shown even when pinned, with the changes marked", () => {
    let board = pinBoard(received(emptyBoard(), first), true);
    board = received(board, more, { force: true });
    expect(board.shown?.token).toBe("t2");
    expect(board.marked).toEqual({ added: [3], gone: [], changed: [] });
  });
});

describe("after the DJ's own decision", () => {
  const three = listing([held(1, "a", 0.9), held(2, "b", 0.5), held(3, "c", 0.7)], [{ id: 1, facts: [1, 2, 3] }], "t1");
  const without2 = listing([held(1, "a", 0.9), held(3, "c", 0.7)], [{ id: 1, facts: [1, 3] }], "t2");

  it("while pinned, the decided rows stay in place, marked, and the new token is the one shown", () => {
    let board = pinBoard(received(emptyBoard(), three), true);
    board = afterDecision(board, without2, [2], "accepted");
    expect(board.shown?.token).toBe("t2");
    const rows = shownGroups(board)[0]!.rows;
    expect(rows.map((r) => [r.fact.fact, r.ghost])).toEqual([[1, null], [2, "accepted"], [3, null]]);
    // Leaving the list lets them go.
    board = pinBoard(board, false);
    expect(shownGroups(board)[0]!.rows.map((r) => r.fact.fact)).toEqual([1, 3]);
  });

  it("a Run whose last row was decided keeps its place while pinned", () => {
    const two = listing([held(1, "a", 0.9), held(5, "e", 0.9)], [{ id: 1, facts: [1] }, { id: 2, facts: [5] }], "t1");
    const after = listing([held(5, "e", 0.9)], [{ id: 2, facts: [5] }], "t2");
    const board = afterDecision(pinBoard(received(emptyBoard(), two), true), after, [1], "rejected");
    expect(shownGroups(board).map((g) => g.run?.id)).toEqual([1, 2]);
  });

  it("when something else changed too, the new listing is held, not shown, while pinned", () => {
    const other = listing([held(1, "a", 0.9), held(3, "c", 0.7), held(4, "d", 0.8)], [{ id: 1, facts: [1, 3, 4] }], "t3");
    let board = pinBoard(received(emptyBoard(), three), true);
    board = afterDecision(board, other, [2], "accepted");
    expect(board.shown?.token).toBe("t1");
    expect(pendingChanges(board)).toEqual({ added: [4], gone: [2], changed: [] });
  });
});

describe("the outcome of a decision", () => {
  it("is done with what was decided", () => {
    expect(outcomeOf({ ok: true, count: 2, decided: [{ fact: 1 }, { fact: 2 }] })).toEqual({ kind: "done", count: 2, facts: [1, 2], rejection: null });
    expect(outcomeOf({ ok: true, count: 1, decided: [{ fact: 4 }], rejection: { entry: 17, run: 3, at: 1 } })).toMatchObject({ kind: "done", rejection: 17 });
  });

  it("a part-way accept is never success: it names what was applied and what was not", () => {
    expect(outcomeOf({ ok: false, error: { code: "review_partial", message: "stopped", applied: [1], notApplied: [2, 3] } }))
      .toEqual({ kind: "partial", message: "stopped", applied: [1], notApplied: [2, 3] });
  });

  it("a refusal keeps its code and words", () => {
    expect(outcomeOf({ ok: false, error: { code: "review_changed", message: "Fact 3 is no longer waiting" } }))
      .toEqual({ kind: "refused", code: "review_changed", message: "Fact 3 is no longer waiting" });
  });
});
