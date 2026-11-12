/**
 * The Intake view (wbx-62, `docs/design/screens/intake`): a queue of tracks
 * to prepare, each with what it still needs, and the selected track in the
 * large player above (the shell's player area grows into it while this view
 * shows; see `Player`'s `large`). Two queues share it, chosen in the tree:
 * Intake (tracks with no Prep colour) and Problems (tracks whose Prep colour
 * is Problem).
 *
 * Both queues are the Library service's (`intake.list`, `intake.problems`);
 * the view keeps nothing but the page it shows, and reads it again after
 * every change to the Library, so an edit made by the CLI or an agent shows
 * at once. Selecting a row loads its track into the deck; the edits that
 * prepare it (title, tags, Energy, Prep colour, "mark Clean and next") are
 * wbx-63 onwards and go in the track panel beside the queue.
 *
 * Reading (`useIntakeQueue`): one page of 200 at a time, re-read on every
 * Library change and when the Library finishes loading, coalesced. A failed
 * refresh keeps the page, marks it stale, offers Retry and tells the status
 * line (`onProblem`).
 *
 * Selection is what the deck actually has: choosing a row asks the deck to
 * load it (replacing what is there, without asking: that is Intake's
 * rhythm); the row shows as chosen while the load is pending, and if it
 * fails (missing audio) the selection goes back to the loaded track and
 * the view says so, the reason being in the status line.
 *
 * Keys: the queue is a listbox. Up and Down (Home, End) move the selection
 * and load the track; they are the queue's while it has the focus and stop
 * there, so the deck and the tree never hear them. Space and Enter belong
 * to whatever control has the focus anywhere in the view, never to the
 * deck (`data-own-activation` on the view and on the empty states); the
 * page buttons sit outside the listbox, so its keys never reach them.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import { getBackend } from "@/ipc/client";
import type { RowDto } from "@/ipc/types";
import { energyOf, intakeActionForKey, intakeChord, nextQueuedIndex, type IntakeAction, type PrepColour } from "@/lib/intakeEdits";
import { needsLine, queuePosition, type IntakeCounts, type IntakeQueueName, type QueuedTrack } from "@/lib/intakeQueue";
import { refusal } from "@/lib/menu";
import type { KeyOverrides, Platform } from "@/lib/shortcuts";
import { useCommands } from "@/store/useCommands";
import { useEventCallback } from "@/store/useEventCallback";
import { IntakeTrackPanel } from "./IntakeTrackPanel";
import styles from "./IntakeView.module.css";
import { useIntakeColours } from "./useIntakeColours";
import { useIntakeQueue } from "./useIntakeQueue";

type Backend = Awaited<ReturnType<typeof getBackend>>;

/** How many queued tracks one page holds: the most a refresh ever reads. */
export const INTAKE_PAGE = 200;

const TITLES: Record<IntakeQueueName, string> = { intake: "Intake", problems: "Problems" };

export interface IntakeViewProps {
  queue: IntakeQueueName;
  /** Shows the other queue, as choosing it in the tree does. */
  onQueue: (queue: IntakeQueueName) => void;
  /** The counts the tab shows, for the empty state's offer of Problems. */
  counts: IntakeCounts | null;
  /** The deck's track: the queue's selection is always what actually loaded. */
  loadedTrackId: string | null;
  /** Loads a queued track into the deck, replacing what is there; `false` when it did not load. */
  onLoad: (row: RowDto) => Promise<boolean>;
  /** A refresh failed (the message, for the status line), or recovered (`null`). */
  onProblem?: ((message: string | null) => void) | undefined;
  /**
   * Writes one Library edit through the window's single edit path (App's
   * `writeEdit`: Library Protection, the status line's report); resolves
   * whether it was written (wbx-63).
   */
  onEdit: (what: string, edit: (backend: Backend) => Promise<unknown>) => Promise<boolean>;
  /** Library Protection is on: why `onEdit` refused, when it says no more. */
  protectLibrary: boolean;
  platform: Platform;
  keyOverrides?: KeyOverrides | undefined;
}

/** When a page was read, as the stale note says it. */
function clock(at: Date | null): string {
  return at ? at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }) : "";
}

export function IntakeView(props: IntakeViewProps) {
  const { queue, onQueue, counts, loadedTrackId, onLoad, onProblem, onEdit, protectLibrary, platform, keyOverrides } = props;
  const t = useTranslation();
  // Only the page shown is read (bounded refreshes); paging moves the offset.
  const [offset, setOffset] = useState(0);
  useEffect(() => { setOffset(0); }, [queue]);
  const { page, failed, readAt, retry } = useIntakeQueue(queue, offset, INTAKE_PAGE);
  // A track asked for and not loaded yet; the newest request wins.
  const [pending, setPending] = useState<string | null>(null);
  const [loadFailed, setLoadFailed] = useState<{ wanted: string; kept: string | null } | null>(null);
  const request = useRef(0);
  const listRef = useRef<HTMLDivElement>(null);
  const { colours, failed: coloursFailed, retry: retryColours } = useIntakeColours();
  const [editFailure, setEditFailure] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);

  // Tell the status line about a failed refresh, and that it recovered.
  const problem = failed !== null && page !== null ? `${t(TITLES[queue])} could not be refreshed: ${failed}` : null;
  useEffect(() => { onProblem?.(problem); }, [problem, onProblem]);
  useEffect(() => () => onProblem?.(null), [onProblem]);

  const shownId = pending ?? loadedTrackId;
  const index = page && shownId !== null ? page.tracks.findIndex((track) => track.id === shownId) : -1;

  /** Asks the deck to load a queued track; resolves whether it loaded. */
  const loadQueued = useCallback(async (track: QueuedTrack, row: RowDto): Promise<boolean> => {
    const ask = ++request.current;
    setPending(track.id);
    setLoadFailed(null);
    const kept = page?.tracks.find((queued) => queued.id === loadedTrackId)?.title ?? null;
    const loaded = await onLoad(row);
    // A later choice supersedes this one, whatever became of it.
    if (ask !== request.current) return loaded;
    setPending(null);
    if (!loaded) setLoadFailed({ wanted: track.title, kept });
    return loaded;
  }, [page, onLoad, loadedTrackId]);

  const choose = useCallback(async (at: number) => {
    const track = page?.tracks[at];
    const row = page?.rows[at];
    if (!track || !row) return;
    setEditFailure(null);
    setNote(null);
    await loadQueued(track, row);
  }, [page, loadQueued]);

  // Keep the chosen row in view as the keys move it.
  useEffect(() => {
    if (shownId === null) return;
    const option = listRef.current?.querySelector<HTMLElement>(`[data-track="${CSS.escape(shownId)}"]`);
    option?.scrollIntoView?.({ block: "nearest" });
  }, [shownId]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!page || page.tracks.length === 0) return;
    const last = page.tracks.length - 1;
    let to: number;
    switch (event.key) {
      case "ArrowDown": to = index < 0 ? 0 : Math.min(last, index + 1); break;
      case "ArrowUp": to = index < 0 ? 0 : Math.max(0, index - 1); break;
      case "Home": to = 0; break;
      case "End": to = last; break;
      case "Enter":
      case " ": to = index < 0 ? 0 : index; break;
      default: return;
    }
    event.preventDefault();
    event.stopPropagation();
    if (to !== index || event.key === "Enter" || event.key === " ") void choose(to);
  };

  const title = t(TITLES[queue]);
  const selected = index >= 0 ? page?.tracks[index] : undefined;
  const selectedRow = index >= 0 ? page?.rows[index] : undefined;
  // Every track of a queue has the queue's colour: none, or Problem.
  const queueColour = queue === "problems" ? (colours?.problem ?? null) : null;

  /**
   * One edit of the chosen track through the window's edit path. A refusal
   * is said in the panel as well as the status line; resolves whether it
   * was written.
   */
  const edit = useCallback(async (what: string, change: (backend: Backend) => Promise<unknown>): Promise<boolean> => {
    let reason: string | null = null;
    const written = await onEdit(what, async (backend) => {
      try {
        return await change(backend);
      } catch (error) {
        reason = error instanceof Error ? error.message : String((error as { message?: unknown } | null)?.message ?? error);
        throw error;
      }
    });
    setEditFailure(written ? null : `${t("Not saved")}: ${reason ?? refusal(protectLibrary)}`);
    return written;
  }, [onEdit, protectLibrary, t]);

  /**
   * Moves to the queued track after the chosen one. `left` says the chosen
   * track has just left the queue (it was given a Prep colour), so a next
   * page starts one earlier than it did.
   */
  const advance = useCallback(async (from: number, left: boolean) => {
    if (!page) return;
    const next = nextQueuedIndex(page.tracks.length, from);
    if (next !== null) {
      const track = page.tracks[next];
      const row = page.rows[next];
      if (track && row) await loadQueued(track, row);
      return;
    }
    if (page.nextOffset === null) {
      setNote(t("That was the last track in the queue."));
      return;
    }
    // The next track is on the next page: show that page and load its first.
    const start = page.nextOffset - (left ? 1 : 0);
    setOffset(start);
    const backend = await getBackend();
    const following = await backend.intakeQueue(queue, start, 1).catch(() => null);
    const track = following?.tracks[0];
    const row = following?.rows[0];
    if (track && row) await loadQueued(track, row);
  }, [page, loadQueued, queue, t]);

  /** Gives the chosen track a Prep colour (or none) and, when that takes it out of this queue, moves on. */
  const setPrepColour = useCallback(async (colour: PrepColour | null): Promise<void> => {
    if (!selected || busyRef.current) return;
    const at = index;
    const id = selected.id;
    busyRef.current = true;
    setBusy(true);
    setNote(null);
    try {
      const what = colour === null ? t("Prep colour cleared.") : `${t("Prep colour")}: ${colour.label}.`;
      const written = await edit(what, (backend) => backend.intakeSetPrepColour(id, colour?.id ?? null));
      const stays = (colour?.id ?? null) === (queueColour?.id ?? null);
      if (written && !stays) await advance(at, true);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }, [selected, index, edit, queueColour, advance, t]);

  const markClean = useCallback(() => {
    if (colours?.clean) void setPrepColour(colours.clean);
    else if (colours) setEditFailure(t("No colour label is named Clean, so there is no Clean to mark."));
  }, [colours, setPrepColour, t]);
  const markProblem = useCallback(() => {
    if (!selected) return;
    if (colours?.problem && queueColour?.id === colours.problem.id) {
      // Already Problem (the Problems queue): only move on.
      void advance(index, false);
    } else if (colours?.problem) {
      void setPrepColour(colours.problem);
    } else if (colours) {
      setEditFailure(t("No colour label is named Problem, so there is no Problem to mark."));
    }
  }, [selected, colours, queueColour, advance, index, setPrepColour, t]);
  const skip = useCallback(() => {
    if (busyRef.current) return;
    setEditFailure(null);
    setNote(null);
    void advance(index, false);
  }, [advance, index]);

  const setEnergy = useCallback((level: number) => {
    if (!selected) return Promise.resolve(false);
    const id = selected.id;
    return edit(level === 0 ? t("Energy cleared.") : `${t("Energy")} ${level}.`, (backend) => backend.edits.setTrackRating([id], level));
  }, [selected, edit, t]);
  const setField = useCallback((field: "title" | "artist", value: string) => {
    if (!selected) return Promise.resolve(false);
    const id = selected.id;
    return edit(field === "title" ? t("Title saved.") : t("Artist saved."), (backend) => backend.edits.setTrackField([id], field, value));
  }, [selected, edit, t]);

  // The view's actions, for the palette and the keys: live while a track is chosen.
  const run = (action: IntakeAction): boolean => {
    if (!selected) return false;
    const level = energyOf(action);
    if (level !== null) { void setEnergy(level); return true; }
    if (action === "intakeMarkClean") markClean();
    else if (action === "intakeMarkProblem") markProblem();
    else skip();
    return true;
  };
  useCommands({
    intakeMarkClean: () => run("intakeMarkClean"),
    intakeMarkProblem: () => run("intakeMarkProblem"),
    intakeSkip: () => run("intakeSkip"),
    intakeEnergy1: () => run("intakeEnergy1"),
    intakeEnergy2: () => run("intakeEnergy2"),
    intakeEnergy3: () => run("intakeEnergy3"),
    intakeEnergy4: () => run("intakeEnergy4"),
    intakeEnergy5: () => run("intakeEnergy5"),
  });

  // Their keys: heard on the way down, ahead of the deck (P and Shift with
  // a digit are deck keys in the Export key map; see `intakeEdits.ts`).
  // A text field keeps its keys; Ctrl+Enter there is the field's own.
  const onWindowKey = useEventCallback((event: globalThis.KeyboardEvent) => {
    const action = intakeActionForKey(event, platform, event.target as HTMLElement | null, keyOverrides);
    if (action === null) return;
    event.preventDefault();
    event.stopPropagation();
    if (!event.repeat) run(action);
  });
  useEffect(() => {
    window.addEventListener("keydown", onWindowKey, true);
    return () => window.removeEventListener("keydown", onWindowKey, true);
  }, [onWindowKey]);

  // Nothing read yet: still reading, or the first read failed (the Library
  // still loading, no service). `library:ready` and Retry read again.
  if (!page) {
    return (
      <div className={styles.emptyState} data-testid="intake-loading" data-own-activation="">
        <h2>{title}</h2>
        <p>{failed ?? t("Reading the queue…")}</p>
        {failed !== null ? <button type="button" className={styles.offer} onClick={retry}>{t("Retry")}</button> : null}
      </div>
    );
  }
  const stale = failed !== null ? (
    <div className={styles.stale} data-testid="intake-stale" role="status">
      <span>{t("Could not be refreshed")}: {failed}. {t("Showing what was read at")} {clock(readAt)}.</span>
      <button type="button" className={styles.retry} onClick={retry}>{t("Retry")}</button>
    </div>
  ) : null;
  if (page.unavailable != null) {
    return (
      <div className={styles.emptyState} data-testid="intake-empty" data-own-activation="">
        <h2>{title}</h2>
        {stale}
        <p>{page.unavailable}</p>
        <button type="button" className={styles.offer} onClick={() => onQueue("intake")}>{t("Show Intake")}</button>
      </div>
    );
  }
  if (page.total === 0) {
    const problems = counts?.problems ?? null;
    return (
      // The offers own Space and Enter: pressing one never plays or pauses the deck.
      <div className={styles.emptyState} data-testid="intake-empty" data-own-activation="">
        <h2>{title}</h2>
        {stale}
        {queue === "intake" ? (
          <>
            <p>{t("Intake is empty: every track has a Prep colour.")}</p>
            <button type="button" className={styles.offer} onClick={() => onQueue("problems")}>
              {t("Show Problems")}{problems !== null ? <span className={styles.offerCount}>{problems}</span> : null}
            </button>
          </>
        ) : (
          <>
            <p>{t("No tracks are marked Problem.")}</p>
            <button type="button" className={styles.offer} onClick={() => onQueue("intake")}>{t("Show Intake")}</button>
          </>
        )}
      </div>
    );
  }

  const first = page.offset + 1;
  const end = page.offset + page.tracks.length;
  return (
    <div
      className={styles.view}
      data-testid="intake-view"
      data-queue={queue}
      data-stale={failed !== null ? "" : undefined}
      // Every control here owns Space and Enter (the page buttons, Retry):
      // none of them ever reaches the deck's Play/Pause or Load.
      data-own-activation=""
    >
      <section className={styles.queue} aria-label={t(`${TITLES[queue]} queue`)}>
        <header className={styles.heading}>
          <h2>{title}</h2>
          <span className={styles.position} data-testid="intake-position">
            {queuePosition(index < 0 ? -1 : page.offset + index, page.total)}
          </span>
        </header>
        {stale}
        {loadFailed !== null ? (
          <p className={styles.loadFailed} data-testid="intake-load-failed" role="status">
            {t("Could not load")} “{loadFailed.wanted}”{loadFailed.kept !== null ? <>; {t("the player still has")} “{loadFailed.kept}”</> : null}.
          </p>
        ) : null}
        <div
          ref={listRef}
          className={styles.rows}
          role="listbox"
          aria-label={t(`${TITLES[queue]} queue`)}
          aria-activedescendant={selected ? optionId(selected.id) : undefined}
          tabIndex={0}
          onKeyDown={onKeyDown}
        >
          {page.tracks.map((track, at) => (
            <QueueRow key={track.id} track={track} selected={at === index} onChoose={() => { void choose(at); }} />
          ))}
        </div>
        {page.offset > 0 || page.nextOffset !== null ? (
          // Outside the listbox, so its keys are the buttons' own.
          <nav className={styles.pages} aria-label={t("Queue pages")}>
            <button type="button" className={styles.more} disabled={page.offset === 0}
              onClick={() => setOffset(Math.max(0, page.offset - INTAKE_PAGE))}>
              {t("Previous")}
            </button>
            <span data-testid="intake-page">{first}–{end} {t("of")} {page.total}</span>
            <button type="button" className={styles.more} disabled={page.nextOffset === null}
              onClick={() => { if (page.nextOffset !== null) setOffset(page.nextOffset); }}>
              {t("Next")}
            </button>
          </nav>
        ) : null}
      </section>
      {selected ? (
        <IntakeTrackPanel
          track={selected}
          energy={selectedRow?.rating ?? 0}
          prepColour={queueColour}
          colours={colours}
          coloursFailed={coloursFailed}
          onRetryColours={retryColours}
          onTitle={(value) => setField("title", value)}
          onArtist={(value) => setField("artist", value)}
          onEnergy={setEnergy}
          onPrepColour={(colour) => { void setPrepColour(colour); }}
          onMarkClean={markClean}
          onMarkProblem={markProblem}
          onSkip={skip}
          busy={busy}
          failure={editFailure}
          note={note}
          keys={{
            markClean: intakeChord("intakeMarkClean", keyOverrides),
            markProblem: intakeChord("intakeMarkProblem", keyOverrides),
            skip: intakeChord("intakeSkip", keyOverrides),
          }}
          platform={platform}
          suggestionsUnavailable={page.suggestionsUnavailable}
        />
      ) : (
        <section className={styles.panel} aria-label={t("Track")} data-testid="intake-panel">
          <p className={styles.hint}>{t("Choose a track in the queue to load it into the player.")}</p>
          {note !== null ? <p className={styles.hint} data-testid="intake-note" role="status">{note}</p> : null}
          {page.suggestionsUnavailable !== null ? (
            <p className={styles.hint} data-testid="intake-suggestions-unavailable">{t("Proposed titles and artists could not be read")}: {page.suggestionsUnavailable}</p>
          ) : null}
        </section>
      )}
    </div>
  );
}

function optionId(trackId: string): string {
  return `intake-track-${trackId}`;
}

function QueueRow({ track, selected, onChoose }: { track: QueuedTrack; selected: boolean; onChoose: () => void }) {
  const t = useTranslation();
  const facts = useMemo(
    () => [track.artist, track.bpm !== null ? String(Math.round(track.bpm)) : "", track.key].filter((part) => part !== "").join(" · "),
    [track],
  );
  return (
    <div
      id={optionId(track.id)}
      className={styles.row}
      role="option"
      aria-selected={selected}
      data-track={track.id}
      // The pointer chooses; the keys are the listbox's.
      onPointerDown={(event) => { if (event.button === 0) onChoose(); }}
    >
      <span className={styles.rowTitle}>{track.title || t("Untitled")}</span>
      <span className={styles.rowFacts}>{facts}</span>
      <span className={styles.rowNeeds} data-testid="intake-needs">{t(needsLine(track))}</span>
    </div>
  );
}
