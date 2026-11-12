/** Beat-grid controls recovered from rekordbox's BeatGridAdjustment/TapButton. */
import { useCallback, useEffect, useRef, useState } from "react";
import type { DeckId, GridEdit, GridEditOptions, GridState } from "@/ipc/types";
import { getBackend } from "@/ipc/client";
import { useTranslation } from "@/i18n";
import { SHIFT_MS, HELD_SHIFT_MS, MAX_STRETCH_MS, tapTempo, tapTimeout, withTap } from "@/lib/gridEdit";
import { deckWrites } from "@/lib/trackWrites";

/**
 * The shortest time between two saves of one held nudge (wbx-64 review 7):
 * the presses in between go out together, so a hold of T seconds makes at
 * most 1 + T / 0.25 file writes and Ledger entries, and is one Undo step.
 */
export const HOLD_SAVE_MS = 250;
/** How long a DJ's consent to replace tempo changes waits for the first tap. */
const CONSENT_WAIT_MS = 10_000;

export interface GridEditorDeck {
  trackId: string | null;
  deck: DeckId;
  state: GridState | null;
  setState: (state: GridState) => void;
  positionMs: () => number;
  readOnly: boolean;
  durationMs?: number;
  isDynamicFrom?: (fromMs: number | null) => boolean;
  confirmDynamic?: () => boolean | Promise<boolean>;
  onError?: ((message: string | null) => void) | undefined;
  /**
   * Called on each shift press with its milliseconds, before the save. The
   * deck plays the shifted grid from here, so the shift sounds at once.
   */
  onNudge?: ((ms: number) => void) | undefined;
}
export interface GridEditorActions {
  state: GridState | null;
  /** True while a shift press is not saved yet. */
  nudging: boolean;
  hasGrid: boolean;
  canEdit: boolean;
  fromMs: number | null;
  tapBpmX100: number | null;
  tap: () => void;
  mark: () => void;
  /**
   * Moves the grid `SHIFT_MS` (a press, a key) or `HELD_SHIFT_MS` (a held
   * pointer's repeat) earlier or later. `join` keeps the press in the hold
   * the last one began (a repeat): one hold is one Undo step.
   */
  shift: (direction: -1 | 1, held?: boolean, join?: boolean) => void;
  stretch: (direction: -1 | 1, held?: boolean) => void;
  double: () => void;
  halve: () => void;
  adjustAll: () => void;
  adjustFrom: () => void;
  align: () => void;
  setBpm: (value: string) => void;
  undo: () => void;
  redo: () => void;
  toggleLock: () => void;
}
function describe(error: unknown): string {
  // Tauri rejects with a bare string when a call never reaches a command
  // (unknown command, refused permission, arguments it cannot read); saying
  // it beats a generic failure nobody can trace (#107).
  if (typeof error === "string" && error.trim()) return error;
  if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string" && error.message.trim()) return error.message;
  return "The beat grid could not be saved.";
}
export function useGridEditor(deck: GridEditorDeck): GridEditorActions {
  const { trackId, deck: deckId, state, setState, positionMs, readOnly, onError, isDynamicFrom, confirmDynamic, onNudge } = deck;
  const t = useTranslation();
  // The playhead can sit up to five seconds before the track starts, but the
  // grid has no time before zero and the command takes unsigned
  // milliseconds: a negative time was refused before it reached the backend,
  // so the edit failed with no reason. The start of the track is the nearest
  // time the grid has.
  const playheadMs = useCallback(() => Math.max(0, Math.round(positionMs())), [positionMs]);
  const hasGrid = trackId !== null && state !== null && state.beats > 0;
  const canEdit = hasGrid && !readOnly && !state.locked;
  const [fromMs, setFromMs] = useState<number | null>(null);
  const [taps, setTaps] = useState<number[]>([]);
  const tapsRef = useRef<number[]>([]);
  const tapAnchor = useRef(0);
  const tapRun = useRef(0);
  const session = useRef(crypto.randomUUID());
  const tapTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const queue = useRef(Promise.resolve());
  // The shift presses not saved yet, by track: batches, one per hold, each
  // queued once. Presses that come while a batch waits its turn join it, so
  // a held button never queues more than one save behind the one in flight;
  // a batch whose hold saved less than HOLD_SAVE_MS ago waits that long
  // first. A batch belongs to the track it was pressed on and is saved
  // there even if the DJ has moved on (wbx-64 review 3).
  const nudges = useRef(new Map<string, { ms: number; transaction: string }[]>());
  const holdSaved = useRef(new Map<string, number>());
  const hold = useRef<string | null>(null);
  const holds = useRef(0);
  const [nudging, setNudging] = useState(false);
  // The DJ agreed, for the next tap run, to replace this track's tempo
  // changes (wbx-64 review 4); `asking` while the question is open.
  const consent = useRef<{ track: string; until: number } | null>(null);
  const asking = useRef(false);
  // The stretch presses not saved yet, the same way: a stretch rewrites the
  // tempo, so each save is a database write and a library reload too, and a
  // held widen/narrow saving every repeat ran on for seconds after release
  // (#196). One batch waits behind the save in flight; its target is the
  // beat under the playhead at its first press.
  const pendingStretch = useRef<{ byMs: number; timeMs: number; fromMs: number | null } | null>(null);
  const currentTrack = useRef(trackId);
  currentTrack.current = trackId;
  const cancelTaps = useCallback(() => {
    if (tapTimer.current) clearTimeout(tapTimer.current);
    tapTimer.current = null;
    tapsRef.current = [];
    setTaps([]);
  }, []);
  useEffect(() => {
    pendingStretch.current = null;
    hold.current = null;
    consent.current = null;
    setNudging(false);
  }, [trackId]);
  useEffect(() => { setFromMs(null); cancelTaps(); return () => { if (tapTimer.current) clearTimeout(tapTimer.current); }; }, [trackId, cancelTaps]);
  useEffect(() => { if (readOnly || state?.locked) cancelTaps(); }, [readOnly, state?.locked, cancelTaps]);

  // A null result is an action that had nothing to send.
  // Every action is made for the track it was asked on, even after the DJ
  // moved on: it was accepted. Only what it answers is shown, and only while
  // that track is loaded. Each is followed per track (`deckWrites`), so
  // Intake waits for it and keeps its failure with that track.
  const run = useCallback((action: (edits: Awaited<ReturnType<typeof getBackend>>["edits"]) => Promise<GridState | null>,
    after?: () => void, done?: () => void) => {
    const track = trackId;
    let settle: (why: string | null) => void = () => {};
    if (track !== null) deckWrites.track(track, "grid", new Promise<string | null>((resolve) => { settle = resolve; }));
    queue.current = queue.current.then(async () => {
      try {
        const backend = await getBackend();
        const next = await action(backend.edits);
        settle(null);
        if (currentTrack.current !== track || next === null) return;
        setState(next);
        after?.();
        onError?.(null);
      } catch (error) {
        settle(describe(error));
        if (currentTrack.current === track) onError?.(describe(error));
      }
      finally { done?.(); }
    });
  }, [trackId, setState, onError]);
  // `take` names the edit when its turn in the queue comes, so presses made
  // while it waited can still join it; null sends nothing.
  const send = useCallback((take: () => { change: GridEdit; transaction?: string | undefined; allowDynamic?: boolean } | null, boundary: number | null, after?: () => void) => {
    if (!canEdit || trackId === null) return;
    const options: GridEditOptions = { deck: deckId };
    if (deck.durationMs !== undefined) options.durationMs = deck.durationMs;
    if (boundary !== null) options.fromMs = boundary;
    run(async edits => {
      const taken = take();
      if (taken === null) return null;
      const { change, transaction } = taken;
      if (transaction) options.transaction = transaction;
      if (taken.allowDynamic) options.allowDynamic = true;
      if ((change.kind === "stretch" || change.kind === "tempo") && isDynamicFrom?.(boundary)) {
        // The backend's dialog, not window.confirm. tauri-plugin-dialog's
        // init script replaces window.confirm with a call to a
        // `plugin:dialog|confirm` command that 2.7 no longer registers, so it
        // rejected with a bare string and no dialog was ever drawn: on Linux
        // every tempo edit over tempo changes failed with "could not be
        // saved" (#107; the macOS edit that did nothing in #86 fits too).
        const question = t("This section has tempo changes. Replace them with a constant tempo?");
        const allowed = await (confirmDynamic?.() ?? (await getBackend()).confirm(question));
        if (!allowed) return state;
        options.allowDynamic = true;
      }
      return edits.gridEdit(trackId, change, options);
    }, after);
  }, [canEdit, trackId, deckId, run, isDynamicFrom, confirmDynamic, state, deck.durationMs, t]);
  const edit = useCallback((change: GridEdit, boundary = fromMs, transaction?: string, after?: () => void, allowDynamic?: boolean) => {
    send(() => ({ change, transaction, ...(allowDynamic ? { allowDynamic } : {}) }), boundary, after);
  }, [send, fromMs]);
  const mark = useCallback(() => { if (fromMs === null) edit({ kind: "downbeat", timeMs: playheadMs() }); }, [edit, fromMs, playheadMs]);
  const shift = useCallback((direction: -1 | 1, held = false, join = held) => {
    if (fromMs !== null || !canEdit || trackId === null) return;
    const ms = direction * (held ? HELD_SHIFT_MS : SHIFT_MS);
    onNudge?.(ms);
    const track = trackId;
    if (!join || hold.current === null) {
      holds.current += 1;
      hold.current = `${session.current}:${deckId}:${track}:nudge:${holds.current}`;
    }
    const transaction = hold.current;
    const batches = nudges.current.get(track) ?? [];
    nudges.current.set(track, batches);
    // Presses that come while a batch waits its turn join it, whichever
    // hold they began: one save, one Undo step.
    const open = batches.at(-1);
    if (open) {
      open.ms += ms;
      return;
    }
    const batch = { ms, transaction };
    batches.push(batch);
    setNudging(true);
    // No deck: the deck already plays the shifted grid, and a save that ends
    // behind newer presses must not put an older grid on its metronome.
    const options: GridEditOptions = { transaction };
    if (deck.durationMs !== undefined) options.durationMs = deck.durationMs;
    run(async edits => {
      // A hold saves at most every HOLD_SAVE_MS; its presses meanwhile join.
      const wait = (holdSaved.current.get(transaction) ?? -Infinity) + HOLD_SAVE_MS - performance.now();
      if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
      const left = nudges.current.get(track) ?? [];
      const at = left.indexOf(batch);
      if (at >= 0) left.splice(at, 1);
      if (left.length === 0) nudges.current.delete(track);
      holdSaved.current.set(transaction, performance.now());
      return batch.ms === 0 ? null : edits.gridEdit(track, { kind: "nudge", ms: batch.ms }, options);
    }, undefined, () => { if (!nudges.current.has(track)) setNudging(false); });
  }, [fromMs, canEdit, trackId, deckId, onNudge, deck.durationMs, run]);
  const stretch = useCallback((direction: -1 | 1, held = false) => {
    if (!canEdit || trackId === null) return;
    const byMs = -direction * (held ? HELD_SHIFT_MS : SHIFT_MS);
    const open = pendingStretch.current;
    // The backend moves the target beat at most MAX_STRETCH_MS a save, so a
    // batch that would pass it starts the next one instead.
    if (open !== null && open.fromMs === fromMs && Math.abs(open.byMs + byMs) <= MAX_STRETCH_MS) {
      open.byMs += byMs;
      return;
    }
    const batch = { byMs, timeMs: playheadMs(), fromMs };
    pendingStretch.current = batch;
    send(() => {
      if (pendingStretch.current === batch) pendingStretch.current = null;
      return batch.byMs === 0 ? null : { change: { kind: "stretch", byMs: batch.byMs, timeMs: batch.timeMs } };
    }, fromMs);
  }, [canEdit, trackId, fromMs, playheadMs, send]);
  const double = useCallback(() => edit({ kind: "double" }), [edit]);
  const halve = useCallback(() => edit({ kind: "halve" }), [edit]);
  const adjustAll = useCallback(() => { if (canEdit) { cancelTaps(); setFromMs(null); } }, [canEdit, cancelTaps]);
  const adjustFrom = useCallback(() => {
    if (!canEdit) return;
    cancelTaps();
    const at = playheadMs();
    edit({ kind: "align", timeMs: at }, at, undefined, () => setFromMs(at));
  }, [canEdit, cancelTaps, playheadMs, edit]);
  const align = useCallback(() => { if (fromMs === null) edit({ kind: "align", timeMs: playheadMs() }); }, [edit, fromMs, playheadMs]);
  const setBpm = useCallback((value: string) => {
    const bpm = Number(value);
    if (!value.trim() || !Number.isFinite(bpm) || bpm < 40 || bpm > 499) { onError?.("Enter a BPM from 40 to 499."); return; }
    edit({ kind: "tempo", bpmX100: Math.round(bpm * 100), anchorMs: fromMs ?? 0 });
  }, [edit, fromMs, onError]);
  const tap = useCallback(() => {
    if (!canEdit || fromMs !== null || trackId === null || asking.current) return;
    const now = performance.now();
    const agreed = consent.current !== null && consent.current.track === trackId && now <= consent.current.until;
    // Over tempo changes a tapped tempo replaces them: asked once, before
    // the run's first tap counts (the same question tempo and stretch ask).
    // No answers no; yes holds for the next tap run.
    if (tapsRef.current.length === 0 && !agreed && isDynamicFrom?.(null)) {
      asking.current = true;
      const track = trackId;
      void (async () => {
        try {
          const question = t("This section has tempo changes. Replace them with a constant tempo?");
          const allowed = await (confirmDynamic?.() ?? (await getBackend()).confirm(question));
          consent.current = allowed && currentTrack.current === track ? { track, until: performance.now() + CONSENT_WAIT_MS } : null;
        } catch {
          consent.current = null;
        } finally {
          asking.current = false;
        }
      })();
      return;
    }
    const next = withTap(tapsRef.current, now);
    if (next.length === 1) {
      tapAnchor.current = playheadMs(); tapRun.current++;
      // A run that has begun keeps its consent until it ends.
      if (agreed && consent.current) consent.current.until = Infinity;
    }
    tapsRef.current = next;
    setTaps(next);
    if (tapTimer.current) clearTimeout(tapTimer.current);
    if (next.length) tapTimer.current = setTimeout(() => { cancelTaps(); consent.current = null; }, tapTimeout(next));
    const bpmX100 = tapTempo(next);
    if (bpmX100 !== null) edit({ kind: "tap", bpm: 60_000 * (next.length - 1) / (next[next.length - 1]! - next[0]!), anchorMs: tapAnchor.current }, null, `${session.current}:${deckId}:${trackId}:${tapRun.current}`, undefined, agreed);
  }, [canEdit, fromMs, trackId, isDynamicFrom, confirmDynamic, t, playheadMs, cancelTaps, edit, deckId]);
  const undo = useCallback(() => { if (hasGrid && !readOnly && trackId !== null) { cancelTaps(); run(edits => edits.gridUndo(trackId, deckId)); } }, [hasGrid, readOnly, trackId, deckId, run, cancelTaps]);
  const redo = useCallback(() => { if (hasGrid && !readOnly && trackId !== null) { cancelTaps(); run(edits => edits.gridRedo(trackId, deckId)); } }, [hasGrid, readOnly, trackId, deckId, run, cancelTaps]);
  const toggleLock = useCallback(() => {
    if (hasGrid && !readOnly && trackId !== null && state !== null) { cancelTaps(); run(edits => edits.gridLock(trackId, !state.locked)); }
  }, [hasGrid, readOnly, trackId, state, cancelTaps, run]);
  return { state, nudging, hasGrid, canEdit, fromMs, tapBpmX100: tapTempo(taps), tap, mark, shift, stretch, double, halve, adjustAll, adjustFrom, align, setBpm, undo, redo, toggleLock };
}
