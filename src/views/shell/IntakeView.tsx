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
 * Keys: the queue is a listbox. Up and Down (Home, End) move the selection
 * and load the track; they are the queue's while it has the focus and stop
 * there, so the deck and the tree never hear them. Space and Enter are the
 * queue's own too (`data-own-activation`).
 */
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import { getBackend } from "@/ipc/client";
import type { RowDto } from "@/ipc/types";
import { needsLine, queuePosition, type IntakeCounts, type IntakeQueueName, type IntakeQueuePage, type QueuedTrack } from "@/lib/intakeQueue";
import { EmptyView } from "./EmptyView";
import styles from "./IntakeView.module.css";

/** How many queued tracks one read asks for; "Show more" asks for the next. */
export const INTAKE_PAGE = 200;

const TITLES: Record<IntakeQueueName, string> = { intake: "Intake", problems: "Problems" };

export interface IntakeViewProps {
  queue: IntakeQueueName;
  /** Shows the other queue, as choosing it in the tree does. */
  onQueue: (queue: IntakeQueueName) => void;
  /** The counts the tab shows, for the empty state's offer of Problems. */
  counts: IntakeCounts | null;
  /** The deck's track, so coming back to the view finds it selected. */
  loadedTrackId: string | null;
  /** Loads a queued track into the deck. */
  onLoad: (row: RowDto) => void;
}

interface Shown {
  queue: IntakeQueueName;
  page: IntakeQueuePage;
}

export function IntakeView(props: IntakeViewProps) {
  const { queue, onQueue, counts, loadedTrackId, onLoad } = props;
  const t = useTranslation();
  const [shown, setShown] = useState<Shown | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(loadedTrackId);
  // How many tracks the DJ has asked to see; a re-read keeps that many.
  const [wanted, setWanted] = useState(INTAKE_PAGE);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => { setWanted(INTAKE_PAGE); }, [queue]);

  // Read the queue now, and again after every change to the Library.
  useEffect(() => {
    let live = true;
    let asked = 0;
    let stop: (() => void) | undefined;
    const read = async () => {
      const ask = ++asked;
      try {
        const backend = await getBackend();
        const page = await backend.intakeQueue(queue, 0, wanted);
        if (!live || ask !== asked) return;
        setShown({ queue, page });
        setFailed(null);
      } catch (error) {
        if (!live || ask !== asked) return;
        setFailed(error instanceof Error ? error.message : String((error as { message?: unknown }).message ?? error));
      }
    };
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stop = backend.onLibraryChanged(() => { void read(); });
      void read();
    })();
    return () => {
      live = false;
      stop?.();
    };
  }, [queue, wanted]);

  const page = shown?.queue === queue ? shown.page : null;
  const index = page && selectedId !== null ? page.tracks.findIndex((track) => track.id === selectedId) : -1;

  const choose = useCallback((at: number) => {
    const track = page?.tracks[at];
    const row = page?.rows[at];
    if (!track || !row) return;
    setSelectedId(track.id);
    onLoad(row);
  }, [page, onLoad]);

  // Keep the chosen row in view as the keys move it.
  useEffect(() => {
    if (selectedId === null) return;
    const option = listRef.current?.querySelector<HTMLElement>(`[data-track="${CSS.escape(selectedId)}"]`);
    option?.scrollIntoView?.({ block: "nearest" });
  }, [selectedId]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!page || page.tracks.length === 0) return;
    const last = page.tracks.length - 1;
    let to: number | null = null;
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
    if (to !== index || event.key === "Enter" || event.key === " ") choose(to);
  };

  const title = t(TITLES[queue]);
  const selected = index >= 0 ? page?.tracks[index] : undefined;

  if (failed !== null && !page) {
    return <EmptyView title={TITLES[queue]} note={failed} />;
  }
  if (page?.unavailable != null) {
    return (
      <div className={styles.emptyState} data-testid="intake-empty">
        <h2>{title}</h2>
        <p>{page.unavailable}</p>
        <button type="button" className={styles.offer} onClick={() => onQueue("intake")}>{t("Show Intake")}</button>
      </div>
    );
  }
  if (page && page.total === 0) {
    const problems = counts?.problems ?? null;
    return (
      <div className={styles.emptyState} data-testid="intake-empty">
        <h2>{title}</h2>
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

  return (
    <div className={styles.view} data-testid="intake-view" data-queue={queue}>
      <section className={styles.queue} aria-label={t(`${TITLES[queue]} queue`)}>
        <header className={styles.heading}>
          <h2>{title}</h2>
          <span className={styles.position} data-testid="intake-position">{page ? queuePosition(index, page.total) : ""}</span>
        </header>
        <div
          ref={listRef}
          className={styles.rows}
          role="listbox"
          aria-label={t(`${TITLES[queue]} queue`)}
          aria-busy={page === null}
          aria-activedescendant={selected ? optionId(selected.id) : undefined}
          tabIndex={0}
          data-own-activation=""
          onKeyDown={onKeyDown}
        >
          {page?.tracks.map((track, at) => (
            <QueueRow key={track.id} track={track} selected={at === index} onChoose={() => choose(at)} />
          ))}
          {page?.nextOffset != null ? (
            <button type="button" className={styles.more} onClick={() => setWanted((count) => count + INTAKE_PAGE)}>
              {t("Show more")} ({page.total - page.tracks.length})
            </button>
          ) : null}
        </div>
      </section>
      <TrackPanel track={selected} />
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

/**
 * The selected track's panel: what is still empty, informing, never
 * blocking. wbx-63 puts the edits here (title and artist in place, Energy,
 * Prep colour, tags, "mark Clean and next").
 */
function TrackPanel({ track }: { track: QueuedTrack | undefined }) {
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
    </section>
  );
}
