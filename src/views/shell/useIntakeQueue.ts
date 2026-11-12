/**
 * One page of the Intake or Problems queue, kept current (wbx-62).
 *
 * Read when shown, after every change to the Library, every cue or grid
 * edit, and when the Library finishes loading, coalesced so that a burst of changes costs at most one
 * more read (`coalescedRefresh`). Only the page shown is ever read: the
 * cost of a refresh is bounded by the page size, however far the DJ pages.
 *
 * A read that fails keeps the page last read and says so (`failed`), so a
 * stale page is never shown as if it were current; `retry` reads again.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import { getBackend } from "@/ipc/client";
import { coalescedRefresh, type Refresher } from "@/lib/coalescedRefresh";
import type { IntakeQueueName, IntakeQueuePage } from "@/lib/intakeQueue";

export interface IntakeQueueState {
  /** The page last read for this queue and offset; `null` before the first read succeeds. */
  page: IntakeQueuePage | null;
  /** Why the latest read failed; `null` when it succeeded. With a `page`, that page is stale. */
  failed: string | null;
  /** When `page` was read. */
  readAt: Date | null;
  retry: () => void;
}

function messageOf(error: unknown): string {
  if (error instanceof Error) return error.message;
  const message = (error as { message?: unknown } | null)?.message;
  return typeof message === "string" ? message : String(error);
}

export function useIntakeQueue(queue: IntakeQueueName, offset: number, limit: number): IntakeQueueState {
  const [state, setState] = useState<{ key: string; page: IntakeQueuePage | null; failed: string | null; readAt: Date | null }>(
    { key: "", page: null, failed: null, readAt: null },
  );
  const key = `${queue}:${offset}`;
  const refresher = useRef<Refresher | null>(null);

  useEffect(() => {
    let live = true;
    const stops: (() => void)[] = [];
    const refresh = coalescedRefresh(async () => {
      const backend = await getBackend();
      try {
        const page = await backend.intakeQueue(queue, offset, limit);
        if (live) setState({ key, page, failed: null, readAt: new Date() });
      } catch (error) {
        const failed = messageOf(error);
        if (live) setState((shown) => (shown.key === key ? { ...shown, failed } : { key, page: null, failed, readAt: null }));
      }
    });
    refresher.current = refresh;
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stops.push(backend.onLibraryChanged(() => refresh.request()));
      stops.push(backend.onLibraryReady(() => refresh.request()));
      // A cue or grid edit is announced on its own, not as a Library change,
      // and changes what a track still needs (cues, grid) (wbx-64).
      stops.push(backend.onCuesChanged(() => refresh.request()));
      stops.push(backend.onGridChanged(() => refresh.request()));
      refresh.request();
    })();
    return () => {
      live = false;
      refresh.stop();
      for (const stop of stops) stop();
    };
  }, [queue, offset, limit, key]);

  const retry = useCallback(() => refresher.current?.request(), []);
  const current = state.key === key;
  return {
    page: current ? state.page : null,
    failed: current ? state.failed : null,
    readAt: current ? state.readAt : null,
    retry,
  };
}
