/**
 * The second list's rows: the target playlist in its own order (wbx-59).
 *
 * Read when a target is chosen, after every change to the Library and when
 * the Library finishes loading, coalesced so a burst of changes costs at
 * most one more read (`coalescedRefresh`). The target is read whole: a
 * reorder sends the playlist's full order, so every row must be known.
 *
 * A read that fails keeps the rows last read and says so (`failed`): the
 * list shows them as stale and nothing acts on them until a read succeeds.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import { getBackend } from "@/ipc/client";
import type { RowDto, ViewSpec } from "@/ipc/types";
import { coalescedRefresh, type Refresher } from "@/lib/coalescedRefresh";

/** Rows per read, inside the backend's response cap. */
const PAGE = 64;

export interface TargetRowsState {
  /** The rows last read for this target; `null` before the first read succeeds. */
  rows: readonly RowDto[] | null;
  /** Why the latest read failed; `null` when it succeeded. With `rows`, they are stale. */
  failed: string | null;
  retry: () => void;
}

function messageOf(error: unknown): string {
  if (error instanceof Error) return error.message;
  const message = (error as { message?: unknown } | null)?.message;
  return typeof message === "string" ? message : String(error);
}

export function targetSpec(playlist: string): ViewSpec {
  return { source: { kind: "playlist", id: playlist }, sort: "trackNo", descending: false, query: "" };
}

export function useTargetRows(playlist: string | null): TargetRowsState {
  const [state, setState] = useState<{ key: string | null; rows: readonly RowDto[] | null; failed: string | null }>(
    { key: null, rows: null, failed: null },
  );
  const refresher = useRef<Refresher | null>(null);

  useEffect(() => {
    if (playlist === null) return undefined;
    let live = true;
    const stops: (() => void)[] = [];
    const refresh = coalescedRefresh(async () => {
      const backend = await getBackend();
      try {
        const view = await backend.openView(targetSpec(playlist));
        const rows: RowDto[] = [];
        for (let offset = 0; offset < view.len; offset += PAGE) {
          rows.push(...await backend.fetchRows(view.viewId, offset, Math.min(PAGE, view.len - offset)));
        }
        if (live) setState({ key: playlist, rows, failed: null });
      } catch (error) {
        const failed = messageOf(error);
        if (live) setState((shown) => (shown.key === playlist ? { ...shown, failed } : { key: playlist, rows: null, failed }));
      }
    });
    refresher.current = refresh;
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stops.push(backend.onLibraryChanged(() => refresh.request()));
      stops.push(backend.onLibraryReady(() => refresh.request()));
      refresh.request();
    })();
    return () => {
      live = false;
      refresh.stop();
      for (const stop of stops) stop();
    };
  }, [playlist]);

  const retry = useCallback(() => refresher.current?.request(), []);
  const current = playlist !== null && state.key === playlist;
  return { rows: current ? state.rows : null, failed: current ? state.failed : null, retry };
}
