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
import { counterStep } from "@/lib/intakeCounter";
import { energyOf, intakeActionForKey, intakeChord, nextQueuedIndex, type IntakeAction, type PrepColour } from "@/lib/intakeEdits";
import { needsLine, queuePosition, type IntakeCounts, type IntakeQueueName, type IntakeQueuePage, type QueuedTrack } from "@/lib/intakeQueue";
import { refusal } from "@/lib/menu";
import type { TagsSet } from "@/lib/tagPanel";
import { TagStore, tagError } from "@/lib/tagStore";
import { formatKey } from "@/lib/preferences";
import type { KeyOverrides, Platform } from "@/lib/shortcuts";
import { useCommands } from "@/store/useCommands";
import { useEventCallback } from "@/store/useEventCallback";
import { usePreferences } from "@/store/usePreferences";
import { IntakeTagPanel, TagFailures, TagPicker } from "./IntakeTagPanel";
import { DeckWriteFailures } from "./DeckWriteFailures";
import { deckWrites } from "@/lib/trackWrites";
import { IntakeTrackPanel } from "./IntakeTrackPanel";
import styles from "./IntakeView.module.css";
import { useIntakeColours } from "./useIntakeColours";
import { useIntakeQueue } from "./useIntakeQueue";

type Backend = Awaited<ReturnType<typeof getBackend>>;

/** What an edit-and-move started on: the track, where it stood, and the load request it began under. */
interface IntakeOperation {
  gen: number;
  id: string;
  page: IntakeQueuePage;
  index: number;
  queue: IntakeQueueName;
}

/** A "mark and next" step: the marked track, its place in its queue, and where the view went. */
interface IntakeMark {
  id: string;
  position: number;
  queue: IntakeQueueName;
  advancedTo: string;
}

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

  // What async work checks before it moves anything: the shown track and
  // the load request it started under. A DJ who chose another track since
  // owns the selection; nothing started before may move it.
  const shownRef = useRef(shownId);
  shownRef.current = shownId;
  const queueRef = useRef(queue);
  queueRef.current = queue;
  const begin = (): IntakeOperation | null => {
    if (!selected || !page) return null;
    return { gen: request.current, id: selected.id, page, index, queue };
  };
  const owns = (op: IntakeOperation) => request.current === op.gen && shownRef.current === op.id;

  /** One operation at a time (a mark, a save then a mark, a skip): the rest wait for it or are ignored. */
  const exclusive = useCallback(async (work: () => Promise<void>) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    try {
      await work();
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }, []);

  /**
   * Moves on from the track `op` started on, if the DJ has not chosen
   * another since: the next row of its page, else the first of the next
   * page (`left`: the track has just left the queue, so that page starts
   * one earlier). Answers the track it loaded.
   */
  const advance = useCallback(async (op: IntakeOperation, left: boolean): Promise<string | null> => {
    const next = nextQueuedIndex(op.page.tracks.length, op.index);
    if (next !== null) {
      const track = op.page.tracks[next];
      const row = op.page.rows[next];
      if (!track || !row || !owns(op)) return null;
      await loadQueued(track, row);
      return track.id;
    }
    if (op.page.nextOffset === null) {
      if (owns(op)) setNote(t("That was the last track in the queue."));
      return null;
    }
    const start = op.page.nextOffset - (left ? 1 : 0);
    const backend = await getBackend();
    const following = await backend.intakeQueue(op.queue, start, 1).catch(() => null);
    const track = following?.tracks[0];
    const row = following?.rows[0];
    // The read took a while: the DJ may have moved on meanwhile.
    if (!track || !row || !owns(op)) return null;
    setOffset(start);
    await loadQueued(track, row);
    return track.id;
  }, [loadQueued, t]);

  /**
   * The half-width counter's previous (`-1`) and next (`1`), wbx-69: what
   * choosing the neighbouring row would do, across a page's edge too (one
   * track read, then the view holds its page). A DJ who chose another
   * track while that read was on its way keeps their choice.
   */
  const step = useEventCallback(async (delta: -1 | 1) => {
    if (!page) return;
    const to = counterStep({ length: page.tracks.length, offset: page.offset, nextOffset: page.nextOffset, index }, delta, INTAKE_PAGE);
    if (to === null) return;
    if (to.kind === "row") { await choose(to.at); return; }
    const op = begin();
    if (!op) return;
    setEditFailure(null);
    setNote(null);
    const backend = await getBackend();
    const read = await backend.intakeQueue(op.queue, to.read, 1).catch(() => null);
    const track = read?.tracks[0];
    const row = read?.rows[0];
    if (!owns(op)) return;
    if (!track || !row) { setNote(t("The neighbouring track could not be read.")); return; }
    setOffset(to.offset);
    await loadQueued(track, row);
  });

  // The tag panel's state and writes (wbx-65), kept here so they outlive
  // the panel of any one track: a mark waits for its track's tag writes,
  // and a failure stays listed after the DJ has moved on. Every click goes
  // through `onEdit` (Library Protection, the status line) to the
  // service's `intake.set_tags`: one step of the shared Undo.
  const myTagComments = usePreferences().advanced.myTagComments;
  const tagEdit = useRef({ onEdit, protectLibrary, myTagComments, t });
  tagEdit.current = { onEdit, protectLibrary, myTagComments, t };
  const [picking, setPicking] = useState(false);
  const [tagStore] = useState(() => new TagStore({
    read: async (track) => (await getBackend()).intakeTagPanel(track),
    write: async (write) => {
      const { onEdit: writeEdit, protectLibrary: locked, myTagComments: comments, t: tr } = tagEdit.current;
      let answer: TagsSet | null = null;
      let reason: unknown = null;
      const written = await writeEdit(tr("Tags saved."), async (backend) => {
        try {
          answer = await backend.intakeSetTags(write.track, write.dimension, write.terms, write.version, write.dropUnmapped, comments);
          return answer;
        } catch (error) {
          reason = error;
          throw error;
        }
      });
      if (!written || answer === null) {
        throw reason instanceof Error ? reason : Object.assign(new Error(reason === null ? refusal(locked) : tagError(reason).message), { detail: reason === null ? "library_protected" : tagError(reason).code });
      }
      return answer;
    },
  }));
  /**
   * Waits for `id`'s tag writes, then its cue and grid writes (wbx-64);
   * says why any failed and answers whether the mark may go on.
   */
  const tagsWritten = useEventCallback(async (op: IntakeOperation): Promise<boolean> => {
    const why = await tagStore.settled(op.id);
    if (why !== null) {
      if (owns(op)) setEditFailure(`${t("Not saved")}: ${t("tags")}: ${why}`);
      return false;
    }
    const deck = await deckWrites.settled(op.id);
    if (deck === null) return true;
    if (owns(op)) setEditFailure(`${t("Not saved")}: ${deck}`);
    return false;
  });

  // Saves of the title and artist on their way: a mark waits for them, and
  // is not made if one was refused.
  const savesOnTheirWay = useRef(new Set<Promise<boolean>>());
  const savesWritten = async () => (await Promise.all([...savesOnTheirWay.current])).every(Boolean);

  // "Mark and next" steps Undo can take back whole: the track, where it
  // stood in its queue, and the track the view moved to.
  const marks = useRef<IntakeMark[]>([]);

  /**
   * Gives `op`'s track a Prep colour (or none). When that takes it out of
   * the queue and the DJ is still on it, moves on, and remembers the step
   * so that its Undo brings the track back.
   */
  const colourThenNext = useCallback(async (op: IntakeOperation, colour: PrepColour | null): Promise<void> => {
    setNote(null);
    if (!(await savesWritten()) || !owns(op)) return;
    if (!(await tagsWritten(op)) || !owns(op)) return;
    const what = colour === null ? t("Prep colour cleared.") : `${t("Prep colour")}: ${colour.label}.`;
    const written = await edit(what, (backend) => backend.intakeSetPrepColour(op.id, colour?.id ?? null));
    const stays = (colour?.id ?? null) === (queueColour?.id ?? null);
    if (!written || stays || !owns(op)) return;
    const position = op.page.offset + op.index;
    const to = await advance(op, true);
    if (to !== null) marks.current.push({ id: op.id, position, queue: op.queue, advancedTo: to });
  }, [edit, queueColour, advance, t, tagsWritten]);

  const setPrepColour = useEventCallback((colour: PrepColour | null) => {
    const op = begin();
    if (op) void exclusive(() => colourThenNext(op, colour));
  });
  const markClean = useEventCallback(() => {
    const op = begin();
    if (!op) return;
    if (colours?.clean) { const clean = colours.clean; void exclusive(() => colourThenNext(op, clean)); }
    else if (colours) setEditFailure(t("No colour label is named Clean, so there is no Clean to mark."));
  });
  /** Ctrl+Enter in a field: its save, and only if it was written and the DJ is still on the track, the mark. */
  const saveThenMark = useEventCallback((save: () => Promise<boolean>) => {
    const op = begin();
    if (!op) return;
    const clean = colours?.clean ?? null;
    void exclusive(async () => {
      const saved = await save();
      if (!saved || !owns(op)) return;
      if (clean) await colourThenNext(op, clean);
      else setEditFailure(t("No colour label is named Clean, so there is no Clean to mark."));
    });
  });
  const markProblem = useEventCallback(() => {
    const op = begin();
    if (!op) return;
    const problem = colours?.problem ?? null;
    if (problem && queueColour?.id === problem.id) {
      // Already Problem (the Problems queue): only move on.
      void exclusive(async () => { if (await tagsWritten(op)) await advance(op, false); });
    } else if (problem) {
      void exclusive(() => colourThenNext(op, problem));
    } else if (colours) {
      setEditFailure(t("No colour label is named Problem, so there is no Problem to mark."));
    }
  });
  const skip = useEventCallback(() => {
    const op = begin();
    if (!op) return;
    setEditFailure(null);
    setNote(null);
    void exclusive(async () => { if (await tagsWritten(op)) await advance(op, false); });
  });

  // An Undo that put a marked track back in its queue takes the whole step
  // back: the track is chosen and loaded again, on its page. Only while the
  // DJ is still where that step left them; otherwise the chain is broken
  // and nothing moves.
  const restoreAfterUndo = useEventCallback(async () => {
    const top = marks.current.at(-1);
    if (!top) return;
    if (top.queue !== queueRef.current || shownRef.current !== top.advancedTo) {
      marks.current = [];
      return;
    }
    const gen = request.current;
    const backend = await getBackend();
    const back = await backend.intakeQueue(top.queue, top.position, 1).catch(() => null);
    const track = back?.tracks[0];
    const row = back?.rows[0];
    // This Undo was of something else, or the DJ moved meanwhile.
    if (!track || !row || track.id !== top.id) return;
    if (request.current !== gen || shownRef.current !== top.advancedTo) return;
    marks.current.pop();
    setOffset(Math.floor(top.position / INTAKE_PAGE) * INTAKE_PAGE);
    setEditFailure(null);
    setNote(null);
    await loadQueued(track, row);
  });
  useEffect(() => {
    let live = true;
    let stop: (() => void) | null = null;
    void getBackend().then((backend) => {
      if (!live) return;
      stop = backend.onEditHistory((history) => {
        // A step back leaves something to redo.
        if (history.canRedo) void restoreAfterUndo();
      });
    });
    return () => { live = false; stop?.(); };
  }, [restoreAfterUndo]);

  const setEnergy = useCallback((level: number) => {
    if (!selected) return Promise.resolve(false);
    const id = selected.id;
    return edit(level === 0 ? t("Energy cleared.") : `${t("Energy")} ${level}.`, (backend) => backend.edits.setTrackRating([id], level));
  }, [selected, edit, t]);
  const setField = useCallback((field: "title" | "artist", value: string) => {
    if (!selected) return Promise.resolve(false);
    const id = selected.id;
    const saving = edit(field === "title" ? t("Title saved.") : t("Artist saved."), (backend) => backend.edits.setTrackField([id], field, value));
    savesOnTheirWay.current.add(saving);
    void saving.finally(() => savesOnTheirWay.current.delete(saving));
    return saving;
  }, [selected, edit, t]);

  // The view's actions, for the palette and the keys: live while a track is chosen.
  const run = (action: IntakeAction): boolean => {
    if (!selected) return false;
    const level = energyOf(action);
    if (level !== null) { void setEnergy(level); return true; }
    if (action === "intakeTag") setPicking(true);
    else if (action === "intakeMarkClean") markClean();
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
    intakeTag: () => run("intakeTag"),
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

  const picker = picking && selected ? <TagPicker store={tagStore} trackId={selected.id} onClose={() => setPicking(false)} /> : null;
  const titleOf = (id: string) => page?.tracks.find((track) => track.id === id)?.title ?? id;
  const tagFailures = (
    <>
      <TagFailures store={tagStore} titleOf={titleOf} shownTrack={shownId} />
      <DeckWriteFailures titleOf={titleOf} shownTrack={shownId} />
    </>
  );

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
        <TagFailures store={tagStore} titleOf={titleOf} shownTrack={null} />
        <DeckWriteFailures titleOf={titleOf} shownTrack={null} />
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
        {/* At half width (wbx-69) this heading is the queue: the position,
            what the chosen track needs, previous and next; the list and
            its pages give way (`IntakeView.module.css`). */}
        <header className={styles.heading} role="group" aria-label={t(`${TITLES[queue]} counter`)} data-track={selected?.id}>
          <h2>{title}</h2>
          <span className={styles.position} data-testid="intake-position">
            {queuePosition(index < 0 ? -1 : page.offset + index, page.total)}
          </span>
          <span className={`${styles.counterOnly} ${styles.counterNeeds}`} data-testid="intake-counter-needs" title={selected ? t(needsLine(selected)) : undefined}>
            {selected ? t(needsLine(selected)) : ""}
          </span>
          <span className={`${styles.counterOnly} ${styles.counterSteps}`}>
            <button type="button" className={styles.step} aria-label={t("Previous track")} title={t("Previous track")}
              disabled={counterStep({ length: page.tracks.length, offset: page.offset, nextOffset: page.nextOffset, index }, -1, INTAKE_PAGE) === null}
              onClick={() => { void step(-1); }}>◀</button>
            <button type="button" className={styles.step} aria-label={t("Next track")} title={t("Next track")}
              disabled={counterStep({ length: page.tracks.length, offset: page.offset, nextOffset: page.nextOffset, index }, 1, INTAKE_PAGE) === null}
              onClick={() => { void step(1); }}>▶</button>
          </span>
        </header>
        {stale}
        {tagFailures}
        {picker}
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
          onPrepColour={setPrepColour}
          onSaveThenMark={saveThenMark}
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
          tags={<IntakeTagPanel store={tagStore} trackId={selected.id} />}
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
