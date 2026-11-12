/**
 * Drives the analysis queue against the backend.
 *
 * A few tracks at a time (`SLOTS` in the queue): each is a decode and a DSP
 * pass, and three in flight finish a batch nearly three times sooner than one
 * while the progress still reads as a count.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { getBackend } from "@/ipc/client";
import { usePreferences } from "./usePreferences";
import type { AnalysisResult } from "@/ipc/types";
import { bpmRangeLimits, type AnalysisPreferences } from "@/lib/preferences";
import {
  cancel as cancelQueue,
  emptyQueue,
  enqueue,
  fail,
  isRunning,
  isWaiting,
  prioritise,
  reset,
  start,
  succeed,
  total,
  type QueueItem,
  type QueueState,
} from "@/lib/queue";

export interface Analysis {
  recordingError: string | null;
  state: QueueState;
  running: boolean;
  /** Tracks are waiting and none is being analysed: a library job holds the run. */
  waiting: boolean;
  total: number;
  add: (items: readonly QueueItem[], settings?: QueueItem["analysis"]) => void;
  /**
   * Analyses a track a deck has just loaded, ahead of anything waiting, with
   * its waveform drawn as the file decodes.
   */
  draw: (item: QueueItem) => void;
  cancel: () => void;
  clear: () => void;
}

export function useAnalysis(
  onAnalysed?: (trackId: string, result: AnalysisResult) => void,
  /** Called once when a run ends, whether it finished, failed or was stopped. */
  onDrained?: () => void,
  preferences?: AnalysisPreferences,
  /**
   * A library job that moves files (Auto Relocate) is running: no track
   * starts until it ends, so none is read from where its file used to be.
   */
  held = false,
): Analysis {
  const stored = usePreferences();
  const storedPreferences = stored.analysis;
  const { mode, concurrentTracks, firstBeatCue, bpmRange, writeKeyTag, detectKey } = preferences ?? storedPreferences;
  // "Write the value to the ID3 tag" is read when a track's analysis is
  // sent, not when it was queued: rekordbox reads `writeKeyInTag` as each
  // analysis completes. Stored notation is independent of the display choice.
  const keyTag = useRef<"classic" | "alphanumeric" | undefined>(undefined);
  const keyNotation = preferences?.keyNotation ?? storedPreferences.keyNotation ?? "classic";
  const keyNotationRef = useRef(keyNotation);
  useEffect(() => {
    keyTag.current = writeKeyTag ? keyNotation : undefined;
    keyNotationRef.current = keyNotation;
  }, [writeKeyTag, keyNotation]);
  const [state, setState] = useState<QueueState>(emptyQueue);
  // The tracks whose request is in flight, so the effect below never sends
  // one twice.
  const inFlight = useRef(new Set<string>());
  const operation = useRef<Promise<string> | null>(null);
  const mounted = useRef(true);
  const [recordingError, setRecordingError] = useState<string | null>(null);
  const finishOperation = useCallback(() => {
    const ending = operation.current;
    operation.current = null;
    if (!ending) return;
    void (async () => {
      const token = await ending.catch(() => null);
      if (token === null) return;
      try {
        const warning = await (await getBackend()).finishLedgerOperation(token);
        if (warning && mounted.current) setRecordingError(warning);
      } catch (error) {
        if (mounted.current) setRecordingError(`Library changes were accepted, but Ledger completion failed: ${String(error)}`);
      }
    })();
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (inFlight.current.size === 0) finishOperation();
    };
  }, [finishOperation]);
  // Whether a run has been going, so the drain fires once at its end.
  const ran = useRef(false);
  const running = isRunning(state);
  useEffect(() => {
    if (running) {
      ran.current = true;
    } else if (ran.current) {
      ran.current = false;
      finishOperation();
      onDrained?.();
    }
  }, [running, onDrained, finishOperation]);

  useEffect(() => {
    // Fill the free slots first, if the run has not been cancelled; the
    // effect runs again on the new state and sends the requests.
    const next = start(state, concurrentTracks, held);
    if (next !== state) {
      setState(next);
      return;
    }
    for (const track of state.running) {
      if (inFlight.current.has(track.id)) continue;
      inFlight.current.add(track.id);
      void (async () => {
        try {
          const backend = await getBackend();
          const trackMode = track.analysis?.mode ?? mode;
          if (!operation.current) {
            setRecordingError(null);
            operation.current = backend.beginLedgerOperation("analysis");
          }
          const token = await operation.current;
          const result = await backend.analyseTrack(track.id, trackMode, track.analysis,
            keyTag.current, track.drawing || undefined,
            keyNotationRef.current === "alphanumeric" ? keyNotationRef.current : undefined, token);
          if (result.recordingWarning) setRecordingError(result.recordingWarning);
          setState((s) => succeed(s, track.id));
          onAnalysed?.(track.id, result);
        } catch (e) {
          setState((s) => fail(s, track.id, e instanceof Error ? e.message : String(e)));
        } finally {
          inFlight.current.delete(track.id);
          if (!mounted.current && inFlight.current.size === 0) finishOperation();
        }
      })();
    }
  }, [state, onAnalysed, mode, concurrentTracks, held, finishOperation]);

  // The preferred settings, for a track queued without the dialog.
  const preferred = useCallback(
    (): NonNullable<QueueItem["analysis"]> => ({ mode, bpmGrid: true, key: detectKey, highPrecision: true, ...bpmRangeLimits(bpmRange), firstBeatCue }),
    [mode, firstBeatCue, bpmRange, detectKey],
  );
  const add = useCallback((items: readonly QueueItem[], settings?: QueueItem["analysis"]) => {
    // Capture settings at enqueue time, including automatic imports. Later
    // preference changes must not alter tracks still waiting in this batch.
    const chosen = settings ?? preferred();
    setState((s) => enqueue(reset(s), items.map(item => ({ ...item, analysis: { ...chosen } }))));
  }, [preferred]);
  const draw = useCallback((item: QueueItem) => {
    setState((s) => prioritise(reset(s), { ...item, analysis: preferred() }));
  }, [preferred]);
  const cancel = useCallback(() => setState(cancelQueue), []);
  const clear = useCallback(() => setState(reset), []);

  // Memoised as a whole: see the note in `useColumns`.
  const waiting = isWaiting(state, held);
  return useMemo(
    () => ({ recordingError, state, running, waiting, total: total(state), add, draw, cancel, clear }),
    [recordingError, state, running, waiting, add, draw, cancel, clear],
  );
}
