/**
 * The Review view (wbx-74): the Facts waiting for the DJ, grouped by Run,
 * and the DJ's accept, reject and reopen through the Library service's own
 * commands (wbx-27). It adds no logic beyond selection and the confidence
 * threshold (`docs/design/screens/review`).
 *
 * What stays true here:
 * - Every accept sends the token of the listing SHOWN. A refusal because
 *   something changed re-reads, shows the new listing with what changed
 *   marked, and waits for the DJ to decide again; nothing is retried.
 * - While the DJ's pointer (or keyboard focus) is on the list, nothing in
 *   it moves: a new listing is held and announced; decided rows stay in
 *   place, marked, until the DJ leaves the list. So a row arriving live
 *   never lands under the pointer, and the count a bulk control states is
 *   exactly the set it sends.
 * - A part-way accept is shown as what was applied and what was not.
 * - A rejection can be reopened here.
 * - Every control owns its activation (`data-own-activation` on the view),
 *   so Space and Enter press it and never reach the deck.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type FocusEvent } from "react";

import { getBackend } from "@/ipc/client";
import type { RowDto } from "@/ipc/types";
import {
  afterDecision, atOrAbove, clearSelected, emptyBoard, fieldText, isLow, pendingChanges,
  pinBoard, reasonText, received, showLatest, shownGroups, sourceText, toggleSelected, valueText,
  type Board, type DecisionOutcome, type ListingChanges, type RejectedFact, type ReviewRun, type ShownGroup,
} from "@/lib/reviewQueue";

import { readReviewNow, requestReviewRead, useReviewRead } from "./reviewQueueStore";
import { useServiceActivity } from "./useServiceActivity";
import styles from "./ReviewView.module.css";

/** The bulk threshold a Run starts with (the design's "Accept ≥ 0.80"). */
export const DEFAULT_THRESHOLD = 0.8;

export interface ReviewViewProps {
  /** Loads a track into the deck and plays it, staying in the view. */
  onListen?: ((row: RowDto) => void) | undefined;
  /** The deck's track, marked on its rows. */
  loadedTrackId?: string | null | undefined;
}

type Verb = "accept" | "reject" | "reopen";

interface Said {
  verb: Verb;
  outcome: DecisionOutcome;
  /** After a refusal: what changed, with the titles of rows now gone. */
  changes?: ListingChanges | undefined;
  goneTitles?: string[] | undefined;
}

const PAST: Record<Verb, string> = { accept: "accepted", reject: "rejected", reopen: "reopened" };

function when(ms: number): string {
  const d = new Date(ms);
  const today = new Date();
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return d.toDateString() === today.toDateString() ? `today ${time}` : `${d.toLocaleDateString([], { month: "short", day: "2-digit" })} ${time}`;
}

function runTitle(run: ReviewRun | null): string {
  return run ? `Run ${run.id} · ${run.description || "no description"}` : "Run not known";
}

function factWord(n: number): string {
  return n === 1 ? "1 Fact" : `${n} Facts`;
}

function changeLine(c: ListingChanges): string {
  const parts: string[] = [];
  if (c.added.length > 0) parts.push(`${c.added.length} new`);
  if (c.changed.length > 0) parts.push(`${c.changed.length} changed`);
  if (c.gone.length > 0) parts.push(`${c.gone.length} gone`);
  return parts.join(", ");
}

export function ReviewView({ onListen, loadedTrackId = null }: ReviewViewProps) {
  const read = useReviewRead(true);
  const [board, setBoard] = useState<Board>(emptyBoard);
  const seen = useRef<unknown>(null);
  const [thresholds, setThresholds] = useState<ReadonlyMap<number | null, number>>(new Map());
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<Said | null>(null);
  const [lastRejection, setLastRejection] = useState<{ entry: number; count: number } | null>(null);
  const [rejectedOpen, setRejectedOpen] = useState(false);
  const [rejected, setRejected] = useState<{ rows: RejectedFact[]; error: string | null } | null>(null);
  const [pointerIn, setPointerIn] = useState(false);
  const [focusIn, setFocusIn] = useState(false);

  // A new read is shown at once, or held while the DJ is on the list.
  useEffect(() => {
    if (read.listing && read.listing !== seen.current) {
      seen.current = read.listing;
      const listing = read.listing;
      setBoard((b) => received(b, listing));
    }
  }, [read.listing]);

  const pinned = pointerIn || focusIn || busy;
  useEffect(() => { setBoard((b) => pinBoard(b, pinned)); }, [pinned]);

  const groups = useMemo(() => shownGroups(board), [board]);
  const pending = pendingChanges(board);
  const shown = board.shown;
  const working = shown?.runs.some((r) => r.finishedAt === null) ?? false;

  const loadRejected = useCallback(async () => {
    try {
      const backend = await getBackend();
      setRejected({ rows: await backend.reviewRejected(), error: null });
    } catch (error) {
      setRejected((r) => ({ rows: r?.rows ?? [], error: error instanceof Error ? error.message : String(error) }));
    }
  }, []);

  const decide = useCallback(async (verb: Verb, facts: number[], rejection: number | null = null) => {
    const before = board.shown;
    if ((verb !== "reopen" && (!before || facts.length === 0)) || busy) return;
    setBusy(true);
    try {
      const backend = await getBackend();
      let outcome: DecisionOutcome;
      try {
        outcome = verb === "accept"
          ? await backend.reviewAccept(facts, before!.token)
          : verb === "reject"
            ? await backend.reviewReject(facts)
            : await backend.reviewReopen(facts, rejection);
      } catch (error) {
        outcome = { kind: "refused", code: "failed", message: error instanceof Error ? error.message : String(error) };
      }
      const fresh = await readReviewNow();
      const listing = fresh.listing;
      if (listing) seen.current = listing;
      if (outcome.kind === "refused" && (outcome.code === "review_changed" || outcome.code === "review_not_waiting") && listing && before) {
        // Shown as it is now, the changes marked; the DJ decides again.
        const next = received(board, listing, { force: true });
        const goneTitles = (next.marked?.gone ?? []).map((id) => before.facts.find((f) => f.fact === id)?.title ?? `Fact ${id}`);
        setBoard((b) => received(b, listing, { force: true }));
        setSaid({ verb, outcome, changes: next.marked ?? undefined, goneTitles });
      } else {
        if (listing) {
          const applied = outcome.kind === "done" ? outcome.facts : outcome.kind === "partial" ? outcome.applied ?? [] : [];
          setBoard((b) => clearSelected(afterDecision(b, listing, applied, PAST[verb])));
        }
        setSaid({ verb, outcome });
      }
      if (verb === "reject" && outcome.kind === "done" && outcome.rejection !== null) {
        setLastRejection({ entry: outcome.rejection, count: outcome.count });
      }
      if (verb === "reopen" && outcome.kind === "done") setLastRejection(null);
      if (verb !== "accept" && (rejectedOpen || verb === "reopen")) void loadRejected();
    } finally {
      setBusy(false);
    }
  }, [board, busy, rejectedOpen, loadRejected]);

  /** The DJ asked for a fresh read: shown at once, wherever the pointer is. */
  const retry = async () => {
    const fresh = await readReviewNow();
    const listing = fresh.listing;
    if (fresh.error === null && listing) {
      seen.current = listing;
      setBoard((b) => showLatest(received(b, listing)));
    }
  };

  const onFocusIn = (event: FocusEvent<HTMLElement>) => {
    const target = event.target;
    // Only keyboard focus pins: a clicked button keeps focus after the pointer leaves.
    setFocusIn(typeof target.matches === "function" && target.matches(":focus-visible"));
  };
  const onFocusOut = (event: FocusEvent<HTMLElement>) => {
    if (!event.currentTarget.contains(event.relatedTarget)) setFocusIn(false);
  };

  const thresholdOf = (run: ReviewRun | null) => thresholds.get(run?.id ?? null) ?? DEFAULT_THRESHOLD;
  const protectedLibrary = shown?.libraryProtected ?? false;
  const total = shown?.facts.length ?? 0;
  const selected = [...board.selected];

  const listen = (track: string) => {
    const row = shown?.decks[track];
    if (row && onListen) onListen(row);
  };

  return (
    <section className={styles.view} data-testid="review-view" data-own-activation="" aria-label="Review">
      {working ? <RunsFinishing /> : null}
      <header className={styles.head}>
        <h2 className={styles.title}><span className={styles.hidden}>Review: </span>Waiting for you</h2>
        <span className={styles.summary} data-testid="review-summary">
          {shown ? `${factWord(total)} in ${groups.filter((g) => g.rows.some((r) => !r.ghost)).length} Run${groups.length === 1 ? "" : "s"}` : "Reading…"}
        </span>
        <span className={styles.readAt}>
          {read.at !== null ? `read ${new Date(read.at).toLocaleTimeString()}` : null}
          {working ? " · a Run is working" : null}
        </span>
      </header>


      {/* The list and its footer pin together: the chosen bar's count and the
          notices are part of what the DJ is acting on. Notices go below the
          rows, so one appearing never moves a row. */}
      <div
        className={styles.body}
        data-pinned={pinned || undefined}
        onPointerEnter={() => setPointerIn(true)}
        onPointerLeave={() => setPointerIn(false)}
        onFocus={onFocusIn}
        onBlur={onFocusOut}
      >
      <div className={styles.list} data-testid="review-list">

        {shown && groups.length === 0 ? (
          <div className={styles.empty} data-testid="review-empty">
            <h3 className={styles.emptyTitle}>Nothing is waiting for you</h3>
            <p className={styles.emptyNote}>When an agent or a tool proposes a value Projection holds back for you, it is listed here by Run.</p>
          </div>
        ) : null}

        {groups.map((group) => (
          <RunGroupView
            key={group.run?.id ?? "unknown"}
            group={group}
            threshold={thresholdOf(group.run)}
            onThreshold={(value) => setThresholds((m) => new Map(m).set(group.run?.id ?? null, value))}
            board={board}
            busy={busy}
            protectedLibrary={protectedLibrary}
            loadedTrackId={loadedTrackId}
            canListen={(track) => onListen !== undefined && shown?.decks[track] !== undefined}
            onListen={listen}
            onToggle={(fact) => setBoard((b) => toggleSelected(b, fact))}
            onDecide={(verb, facts) => void decide(verb, facts)}
          />
        ))}
      </div>
      <footer className={styles.foot} data-testid="review-foot">
        {pending ? (
          <div className={styles.pending} role="status" data-testid="review-pending">
            <span>Changed while you were here: {changeLine(pending)}. Shown when you leave the list.</span>
            <button type="button" className={styles.button} onClick={() => setBoard(showLatest)}>Show now</button>
          </div>
        ) : null}
        {selected.length > 0 ? (
          <div className={styles.chosen} data-testid="review-chosen">
            <span>{factWord(selected.length)} chosen</span>
            <button type="button" className={styles.accept} disabled={busy || protectedLibrary} onClick={() => void decide("accept", selected)}>
              Accept chosen · {selected.length}
            </button>
            <button type="button" className={styles.button} disabled={busy} onClick={() => void decide("reject", selected)}>
              Reject chosen · {selected.length}
            </button>
            <button type="button" className={styles.button} onClick={() => setBoard(clearSelected)}>Clear</button>
          </div>
        ) : null}
        {read.error !== null ? (
          <div className={styles.failed} role="alert" data-testid="review-read-failed">
            <span>Could not read the Facts waiting for you: {read.error}{shown ? " What is listed below may be out of date." : ""}</span>
            <button type="button" className={styles.button} onClick={() => void retry()}>Retry</button>
          </div>
        ) : null}

        {protectedLibrary ? (
          <p className={styles.note} data-testid="review-protected">
            Library Protection is on: nothing can be accepted until you turn it off. Rejecting writes only the Sidecar.
          </p>
        ) : null}

        {said ? <Outcome said={said} onDismiss={() => setSaid(null)} /> : null}

        {lastRejection ? (
          <div className={styles.note} data-testid="review-last-rejection">
            <span>Rejected {factWord(lastRejection.count)}.</span>
            <button type="button" className={styles.button} disabled={busy} onClick={() => void decide("reopen", [], lastRejection.entry)}>
              Reopen these {lastRejection.count}
            </button>
          </div>
        ) : null}

        <details
          className={styles.rejected}
          data-testid="review-rejected"
          onToggle={(e) => {
            const open = (e.currentTarget).open;
            setRejectedOpen(open);
            if (open) void loadRejected();
          }}
        >
          <summary className={styles.rejectedSummary}>Rejected suggestions</summary>
          {rejected?.error ? <p role="alert" className={styles.failed}>Could not read the rejected suggestions: {rejected.error}</p> : null}
          {rejected && rejected.rows.length === 0 && !rejected.error ? <p className={styles.emptyNote}>Nothing is rejected.</p> : null}
          <ul className={styles.rejectedList}>
            {rejected?.rows.map((r) => (
              <li key={r.fact} className={styles.rejectedRow} data-fact={r.fact}>
                <span>{shown?.decks[r.track]?.title ?? `Track ${r.track}`}</span>
                <span>{fieldText(r.field)} {valueText(r.field, r.value)}</span>
                <span className={styles.dim}>{sourceText(r.source)} · {r.confidence.toFixed(2)}</span>
                <button type="button" className={styles.button} disabled={busy} onClick={() => void decide("reopen", [r.fact])}
                  aria-label={`Reopen ${fieldText(r.field)} ${valueText(r.field, r.value)} for track ${r.track}`}>
                  Reopen
                </button>
              </li>
            ))}
          </ul>
        </details>
      </footer>
      </div>
    </section>
  );
}

/**
 * While a shown Run is working: reads the queue again when the service's
 * active Runs change, since a Run finishing sends no event.
 */
function RunsFinishing() {
  const activity = useServiceActivity();
  const active = activity?.runs?.map((r) => r.id).join(",") ?? null;
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    requestReviewRead();
  }, [active]);
  return null;
}

function Outcome({ said, onDismiss }: { said: Said; onDismiss: () => void }) {
  const { verb, outcome } = said;
  if (outcome.kind === "done") {
    return (
      <div className={styles.done} role="status" data-testid="review-outcome" data-outcome="done">
        <span>{PAST[verb][0]!.toUpperCase() + PAST[verb].slice(1)} {factWord(outcome.count)}.</span>
        <button type="button" className={styles.button} onClick={onDismiss}>Dismiss</button>
      </div>
    );
  }
  if (outcome.kind === "partial") {
    return (
      <div className={styles.failed} role="alert" data-testid="review-outcome" data-outcome="partial">
        <strong>The accept stopped part-way: not everything was applied.</strong>
        <span data-testid="review-applied">Applied: {outcome.applied ? (outcome.applied.length ? outcome.applied.map((f) => `Fact ${f}`).join(", ") : "none") : "see below"}</span>
        <span data-testid="review-not-applied">Not applied, still waiting: {outcome.notApplied ? (outcome.notApplied.length ? outcome.notApplied.map((f) => `Fact ${f}`).join(", ") : "none") : "see the list"}</span>
        <span className={styles.dim}>{outcome.message}</span>
        <button type="button" className={styles.button} onClick={onDismiss}>Dismiss</button>
      </div>
    );
  }
  const changed = outcome.code === "review_changed" || outcome.code === "review_not_waiting";
  return (
    <div className={styles.failed} role="alert" data-testid="review-outcome" data-outcome="refused" data-code={outcome.code}>
      <strong>
        {changed
          ? `Nothing was ${PAST[verb]}: what you were shown has changed. Decide again on the list as it is now.`
          : outcome.code === "library_protected"
            ? `Nothing was ${PAST[verb]}: Library Protection is on.`
            : `Nothing was ${PAST[verb]}.`}
      </strong>
      {said.changes ? <span data-testid="review-changes">{changeLine(said.changes) || "no row differs now"}{said.goneTitles?.length ? `: gone ${said.goneTitles.join(", ")}` : ""}</span> : null}
      <span className={styles.dim}>{outcome.message}</span>
      <button type="button" className={styles.button} onClick={onDismiss}>Dismiss</button>
    </div>
  );
}

interface RunGroupProps {
  group: ShownGroup;
  threshold: number;
  onThreshold: (value: number) => void;
  board: Board;
  busy: boolean;
  protectedLibrary: boolean;
  loadedTrackId: string | null;
  canListen: (track: string) => boolean;
  onListen: (track: string) => void;
  onToggle: (fact: number) => void;
  onDecide: (verb: "accept" | "reject", facts: number[]) => void;
}

function RunGroupView({ group, threshold, onThreshold, board, busy, protectedLibrary, loadedTrackId, canListen, onListen, onToggle, onDecide }: RunGroupProps) {
  const { run } = group;
  const live = group.rows.filter((r) => !r.ghost).map((r) => r.fact);
  // Computed from what is drawn; the same array is what the control sends.
  const above = atOrAbove(live, threshold);
  const held = live.filter((f) => f.reason !== "waits_for_dj");
  const label = runTitle(run);
  const marked = board.marked;
  return (
    <section className={styles.run} role="group" aria-label={label} data-run={run?.id ?? ""}>
      <div className={styles.runHead}>
        <span className={styles.runInfo} title={label}>
        <span className={styles.runTitle}>{label}</span>
        <span className={styles.dim}>
          {" "}{run ? `${sourceText(run.source)} · ${when(run.startedAt)} · ` : ""}
          {live.length} waiting
          {run && run.finishedAt === null ? <span className={styles.working} data-testid="review-run-working"> · working</span> : null}
          {held.length > 0 ? ` · ${held.length} held: ${[...new Set(held.map((f) => reasonText(f).replace(/^held: /, "")))].join(", ")}` : ""}
        </span>
        </span>
        <span className={styles.bulk}>
          <label className={styles.threshold}>
            <span>≥</span>
            <input
              type="number" min={0} max={1} step={0.05} value={threshold.toFixed(2)}
              aria-label={`Confidence for ${label}`}
              onChange={(e) => {
                const v = Number(e.currentTarget.value);
                if (Number.isFinite(v)) onThreshold(Math.max(0, Math.min(1, v)));
              }}
            />
          </label>
          <button type="button" className={styles.accept} data-testid="review-bulk-accept"
            disabled={busy || protectedLibrary || above.length === 0}
            onClick={() => onDecide("accept", above)}>
            Accept ≥ {threshold.toFixed(2)} · {above.length}
          </button>
          <button type="button" className={styles.button} data-testid="review-bulk-reject"
            disabled={busy || above.length === 0}
            onClick={() => onDecide("reject", above)}>
            Reject ≥ {threshold.toFixed(2)} · {above.length}
          </button>
        </span>
      </div>
      <div className={styles.table} role="table" aria-label={`Facts waiting in ${label}`}>
        <div className={styles.header} role="row">
          <span role="columnheader" className={styles.pick}><span className={styles.hidden}>Choose</span></span>
          <span role="columnheader">Track</span>
          <span role="columnheader">Field</span>
          <span role="columnheader" className={styles.now}>Now</span>
          <span role="columnheader">Proposed</span>
          <span role="columnheader">Sure</span>
          <span role="columnheader" className={styles.why}>Why</span>
          <span role="columnheader"><span className={styles.hidden}>Decide</span></span>
        </div>
        {group.rows.map(({ fact: f, ghost }) => {
          const low = isLow(f.confidence);
          const mark = marked?.added.includes(f.fact) ? "added" : marked?.changed.includes(f.fact) ? "changed" : undefined;
          const what = `${fieldText(f.field)} ${valueText(f.field, f.after)} for ${f.title || f.track}`;
          return (
            <div key={f.fact} role="row" className={styles.row} data-fact={f.fact} data-track={f.track}
              data-low={low || undefined} data-ghost={ghost ?? undefined} data-marked={mark}
              data-loaded={loadedTrackId === f.track || undefined} aria-selected={board.selected.has(f.fact)}>
              <span role="cell" className={styles.pick}>
                {ghost ? null : (
                  <input type="checkbox" checked={board.selected.has(f.fact)} disabled={busy}
                    aria-label={`Choose ${what}`} onChange={() => onToggle(f.fact)} />
                )}
              </span>
              <span role="cell" className={styles.track} title={f.title}>{f.title || f.track}</span>
              <span role="cell" className={styles.dim}>{fieldText(f.field)}</span>
              <span role="cell" className={`${styles.dim} ${styles.now}`}>{valueText(f.field, f.before)}</span>
              <span role="cell" className={styles.proposed}>{valueText(f.field, f.after)}</span>
              <span role="cell" className={styles.sure}>
                <span className={styles.bar} aria-hidden="true"><span className={styles.fill} style={{ width: `${Math.round(f.confidence * 100)}%` }} /></span>
                <span>{f.confidence.toFixed(2)}</span>
                {low ? <span className={styles.lowTag}>low</span> : null}
              </span>
              <span role="cell" className={`${styles.dim} ${styles.why}`}>{reasonText(f)}</span>
              <span role="cell" className={styles.decide}>
                {ghost ? (
                  <span className={styles.ghostTag}>{ghost}</span>
                ) : (
                  <>
                    <button type="button" className={styles.icon} disabled={!canListen(f.track)}
                      aria-label={`Listen to ${f.title || f.track}`} title="Load into the deck and play"
                      onClick={() => onListen(f.track)}>▶</button>
                    <button type="button" className={styles.yes} disabled={busy || protectedLibrary}
                      aria-label={`Accept ${what}`} title={protectedLibrary ? "Library Protection is on" : "Accept"}
                      onClick={() => onDecide("accept", [f.fact])}>✓</button>
                    <button type="button" className={styles.no} disabled={busy}
                      aria-label={`Reject ${what}`} title="Reject"
                      onClick={() => onDecide("reject", [f.fact])}>✕</button>
                  </>
                )}
              </span>
            </div>
          );
        })}
      </div>
    </section>
  );
}
