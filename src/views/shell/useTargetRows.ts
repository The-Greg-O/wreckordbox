/**
 * The second list's rows: the target playlist's entries in its own order,
 * each with its own identity (wbx-59).
 *
 * Two reads: the playlist's rows as the list shows tracks (`openView` /
 * `fetchRows`), and its entries (`playlistEntries`), matched by place. A
 * track that is in the playlist twice is two entries, so a reorder can name
 * one of them. When the two reads disagree (an edit landed between them)
 * they are read again; a second disagreement is a failed read.
 *
 * Read when a target is chosen, after every change to the Library and when
 * the Library finishes loading, coalesced so a burst of changes costs at
 * most one more read (`coalescedRefresh`). The target is read whole: a
 * reorder sends the playlist's full order, so every entry must be known.
 *
 * A read that fails keeps the rows last read and says so (`failed`): the
 * list shows them as stale and nothing is reordered until a read succeeds.
 * `readNo` numbers reads by when they started, so a caller can tell a read
 * made after its write from one already under way.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import { getBackend } from "@/ipc/client";
import type { RowDto, ViewSpec } from "@/ipc/types";
import { coalescedRefresh, type Refresher } from "@/lib/coalescedRefresh";

/** Rows per read, inside the backend's response cap. */
const PAGE = 64;

export interface TargetRow {
  /** The entry's own identity in the playlist. */
  entryId: string;
  row: RowDto;
}

export interface TargetRowsState {
  /** The entries last read for this target; `null` before the first read succeeds. */
  rows: readonly TargetRow[] | null;
  /** Why the latest read failed; `null` when it succeeded. With `rows`, they are stale. */
  failed: string | null;
  /** The number of the read `rows` came from: reads are numbered as they start. */
  readNo: number;
  /** How many reads have started: the next read is numbered past this. */
  readsStarted: () => number;
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

async function readOnce(playlist: string): Promise<TargetRow[] | null> {
  const backend = await getBackend();
  const view = await backend.openView(targetSpec(playlist));
  const rows: RowDto[] = [];
  for (let offset = 0; offset < view.len; offset += PAGE) {
    rows.push(...await backend.fetchRows(view.viewId, offset, Math.min(PAGE, view.len - offset)));
  }
  const entries = await backend.playlistEntries(playlist);
  if (entries.length !== rows.length || entries.some((entry, i) => entry.trackId !== rows[i]!.id)) return null;
  return rows.map((row, i) => ({ entryId: entries[i]!.entryId, row }));
}

export function useTargetRows(playlist: string | null): TargetRowsState {
  const [state, setState] = useState<{ key: string | null; rows: readonly TargetRow[] | null; failed: string | null; readNo: number }>(
    { key: null, rows: null, failed: null, readNo: 0 },
  );
  const refresher = useRef<Refresher | null>(null);
  const started = useRef(0);

  useEffect(() => {
    if (playlist === null) return undefined;
    let live = true;
    const stops: (() => void)[] = [];
    const refresh = coalescedRefresh(async () => {
      started.current += 1;
      const readNo = started.current;
      try {
        const rows = (await readOnce(playlist)) ?? (await readOnce(playlist));
        if (rows === null) throw new Error("The target changed while it was being read.");
        if (live) setState({ key: playlist, rows, failed: null, readNo });
      } catch (error) {
        const failed = messageOf(error);
        if (live) {
          setState((shown) => (shown.key === playlist ? { ...shown, failed } : { key: playlist, rows: null, failed, readNo }));
        }
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
  const readsStarted = useCallback(() => started.current, []);
  const current = playlist !== null && state.key === playlist;
  return {
    rows: current ? state.rows : null,
    failed: current ? state.failed : null,
    readNo: current ? state.readNo : 0,
    readsStarted,
    retry,
  };
}
