/**
 * The window's Stem splits (wbx-67), one queue for the whole shell: the
 * large player's Stems control asks it, the status line shows it. Created
 * on first use, against the backend, and kept for the page's life, so a
 * split carries on whichever view or track is shown.
 */
import { useEffect, useRef, useState, useSyncExternalStore } from "react";

import { getBackend } from "@/ipc/client";
import { createStemSplits, type Split, type StemSplits } from "@/lib/stemSplits";
import type { StemsStatus } from "@/lib/stemsStatus";
import { coalescedRefresh } from "@/lib/coalescedRefresh";

let splits: StemSplits | null = null;
let unsubscribeProgress: (() => void) | null = null;

export function stemSplits(): StemSplits {
  splits ??= createStemSplits({
    separate: async (id, again) => (await getBackend()).stemsSeparate(id, again),
    onProgress: (listener) => {
      let live = true;
      void getBackend().then((backend) => {
        if (live) unsubscribeProgress = backend.onStemsProgress(listener);
      });
      return () => { live = false; unsubscribeProgress?.(); };
    },
  });
  return splits;
}

const EMPTY: readonly Split[] = [];

/** Every split the window knows of. */
export function useStemSplits(): readonly Split[] {
  return useSyncExternalStore(
    (listener) => stemSplits().subscribe(listener),
    () => splits?.all() ?? EMPTY,
    () => EMPTY,
  );
}

/** One track's split, if the window has asked for one. */
export function useStemSplit(id: string | null): Split | null {
  const all = useStemSplits();
  return id === null ? null : all.find((split) => split.trackId === id) ?? null;
}

export type StemsRead =
  | { status: "reading" }
  | { status: "read"; value: StemsStatus }
  | { status: "failed"; message: string };

/**
 * A track's Stems as the service reports them: read when the track
 * changes, when the Sidecar changes for it (any client), when the window's
 * own split for it ends, and when the Library becomes ready. Reads are
 * coalesced (one in flight, one pending). An answer for a track no longer
 * shown is dropped; a failed read is shown as a failure, not as the last
 * state read.
 */
export function useStemsStatus(id: string | null, splitPhase: Split["phase"] | null): { read: StemsRead; retry: () => void } {
  const [read, setRead] = useState<StemsRead>({ status: "reading" });
  const refresh = useRef<(() => void) | null>(null);
  useEffect(() => {
    if (id === null) return;
    let live = true;
    setRead({ status: "reading" });
    const reader = coalescedRefresh(async () => {
      const backend = await getBackend();
      try {
        const value = await backend.stemsStatus(id);
        if (live) setRead({ status: "read", value });
      } catch (error) {
        if (live) setRead({ status: "failed", message: error instanceof Error ? error.message : String(error) });
      }
    });
    refresh.current = () => { reader.request(); };
    const unsubscribers: (() => void)[] = [];
    void getBackend().then((backend) => {
      if (!live) return;
      unsubscribers.push(backend.onSidecarChanged((change) => { if (change.trackId === id && change.kind === "stems") reader.request(); }));
      unsubscribers.push(backend.onLibraryReady(() => { reader.request(); }));
    });
    reader.request();
    return () => {
      live = false;
      refresh.current = null;
      reader.stop();
      for (const unsubscribe of unsubscribers) unsubscribe();
    };
  }, [id]);
  // The window's own split for this track ended (done or failed): read again.
  const phase = useRef(splitPhase);
  useEffect(() => {
    const was = phase.current;
    phase.current = splitPhase;
    if (was !== splitPhase && (splitPhase === "done" || splitPhase === "failed")) refresh.current?.();
  }, [splitPhase]);
  return { read, retry: () => { refresh.current?.(); } };
}
