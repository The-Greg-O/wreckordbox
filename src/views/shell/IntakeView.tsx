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
import type { RowDto } from "@/ipc/types";
import { needsLine, queuePosition, type IntakeCounts, type IntakeQueueName, type QueuedTrack } from "@/lib/intakeQueue";
import { formatKey } from "@/lib/preferences";
import { usePreferences } from "@/store/usePreferences";
import styles from "./IntakeView.module.css";
import { useIntakeQueue } from "./useIntakeQueue";

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
}

/** When a page was read, as the stale note says it. */
function clock(at: Date | null): string {
  return at ? at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }) : "";
}

export function IntakeView(props: IntakeViewProps) {
  const { queue, onQueue, counts, loadedTrackId, onLoad, onProblem } = props;
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

  // Tell the status line about a failed refresh, and that it recovered.
  const problem = failed !== null && page !== null ? `${t(TITLES[queue])} could not be refreshed: ${failed}` : null;
  useEffect(() => { onProblem?.(problem); }, [problem, onProblem]);
  useEffect(() => () => onProblem?.(null), [onProblem]);

  const shownId = pending ?? loadedTrackId;
  const index = page && shownId !== null ? page.tracks.findIndex((track) => track.id === shownId) : -1;

  const choose = useCallback(async (at: number) => {
    const track = page?.tracks[at];
    const row = page?.rows[at];
    if (!track || !row) return;
    const ask = ++request.current;
    setPending(track.id);
    setLoadFailed(null);
    const kept = page?.tracks.find((queued) => queued.id === loadedTrackId)?.title ?? null;
    const loaded = await onLoad(row);
    // A later choice supersedes this one, whatever became of it.
    if (ask !== request.current) return;
    setPending(null);
    if (!loaded) setLoadFailed({ wanted: track.title, kept });
  }, [page, onLoad, loadedTrackId]);

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
      <TrackPanel track={selected} suggestionsUnavailable={page.suggestionsUnavailable} />
    </div>
  );
}

function optionId(trackId: string): string {
  return `intake-track-${trackId}`;
}

function QueueRow({ track, selected, onChoose }: { track: QueuedTrack; selected: boolean; onChoose: () => void }) {
  const t = useTranslation();
  const { keyDisplay } = usePreferences().view;
  const facts = useMemo(
    () => [track.artist, track.bpm !== null ? String(Math.round(track.bpm)) : "", formatKey(track.key, keyDisplay)].filter((part) => part !== "").join(" · "),
    [track, keyDisplay],
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

/**
 * The selected track's panel: what is still empty, informing, never
 * blocking. wbx-63 puts the edits here (title and artist in place, Energy,
 * Prep colour, tags, "mark Clean and next").
 */
function TrackPanel({ track, suggestionsUnavailable }: { track: QueuedTrack | undefined; suggestionsUnavailable: string | null }) {
  const t = useTranslation();
  if (!track) {
    return (
      <section className={styles.panel} aria-label={t("Track")} data-testid="intake-panel">
        <p className={styles.hint}>{t("Choose a track in the queue to load it into the player.")}</p>
      </section>
    );
  }
  return (
    <section className={styles.panel} aria-label={t("Track")} data-testid="intake-panel">
      <dl className={styles.fields}>
        <dt>{t("Title")}</dt><dd data-testid="intake-panel-title">{track.title}</dd>
        <dt>{t("Artist")}</dt><dd>{track.artist}</dd>
      </dl>
      {track.suggestions.length > 0 ? (
        <ul className={styles.suggestions} aria-label={t("Proposed")}>
          {track.suggestions.map((suggestion) => (
            <li key={`${suggestion.field}:${suggestion.value}`}>
              <span className={styles.label}>{t(suggestion.field)}</span> {suggestion.value}
              <span className={styles.source}> {suggestion.source} · {suggestion.confidence.toFixed(2)}</span>
            </li>
          ))}
        </ul>
      ) : null}
      <p className={styles.open} data-testid="intake-panel-needs">{t(needsLine(track))}</p>
      {suggestionsUnavailable !== null ? (
        <p className={styles.hint} data-testid="intake-suggestions-unavailable">{t("Proposed titles and artists could not be read")}: {suggestionsUnavailable}</p>
      ) : null}
    </section>
  );
}
