/**
 * The Intake tag panel's state (wbx-65): the service's description of the
 * panel with the shown track's tags, kept current, and the DJ's clicks
 * written one at a time.
 *
 * Reading: when the track changes, after every Library change, when the
 * Library finishes loading and after the panel's own writes, coalesced
 * (`coalescedRefresh`): a burst of changes costs at most one more read. A
 * read that fails keeps what was shown and says so (`failed`).
 *
 * Writing: each click sets what the DJ wants a dimension to hold on that
 * track (`wanted`), shown at once. One worker writes the difference
 * between what is wanted and what is known written, one write at a time:
 * a deck-facing dimension one MyTag tick per step (each its own Undo), a
 * Fact dimension its whole list at once, so clicks made while a write is
 * on its way become one write. A click is always for the track it was
 * made on, whichever track is shown when it lands. A write that fails
 * drops what was wanted for that dimension and says why (`failure`).
 */
import { useCallback, useEffect, useRef, useState } from "react";

import { getBackend } from "@/ipc/client";
import { coalescedRefresh, type Refresher } from "@/lib/coalescedRefresh";
import { toggledTerms, type PanelDimension, type TagPanel } from "@/lib/tagPanel";

export interface TagWriters {
  /** A deck-facing tag on or off: the window's MyTag edit. Resolves why it was not written, or `null`. */
  putMyTag: (track: string, myTag: string, on: boolean) => Promise<string | null>;
  /** A Fact dimension's whole list. Resolves why it was not written, or `null`. */
  setFact: (track: string, dimension: string, terms: readonly string[]) => Promise<string | null>;
}

interface Wanted {
  track: string;
  dimension: PanelDimension;
  /** What the DJ has asked for. */
  wanted: string[];
  /** What is known written (the read it started from, then each write). */
  written: string[];
}

export interface TagPanelState {
  panel: TagPanel | null;
  /** Why the latest read failed; with a `panel`, what is shown is stale. */
  failed: string | null;
  /** Why the latest write was not made; `null` after one that was. */
  failure: string | null;
  /** The tags shown for `dimension` on the shown track: what was asked for, else what was read. */
  shown: (dimension: PanelDimension) => string[];
  toggle: (dimension: PanelDimension, tag: string) => void;
  retry: () => void;
}

function messageOf(error: unknown): string {
  if (error instanceof Error) return error.message;
  const message = (error as { message?: unknown } | null)?.message;
  return typeof message === "string" ? message : String(error);
}

const same = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((x, i) => x === b[i]);
const keyOf = (track: string, dimension: string) => `${track}\n${dimension}`;

export function useTagPanel(trackId: string, writers: TagWriters): TagPanelState {
  const [state, setState] = useState<{ track: string; panel: TagPanel | null; failed: string | null }>(
    { track: "", panel: null, failed: null },
  );
  const [failure, setFailure] = useState<string | null>(null);
  const [, setTick] = useState(0);
  const rerender = useCallback(() => setTick((n) => n + 1), []);
  const wanted = useRef(new Map<string, Wanted>());
  const writing = useRef(false);
  // Writes finished, and the count a read started at: a read that began
  // after the last write, with nothing left to write, is the truth again.
  const writesDone = useRef(0);
  const refresher = useRef<Refresher | null>(null);
  const writersRef = useRef(writers);
  writersRef.current = writers;

  useEffect(() => {
    let live = true;
    const stops: (() => void)[] = [];
    const refresh = coalescedRefresh(async () => {
      const startedAt = writesDone.current;
      const backend = await getBackend();
      try {
        const panel = await backend.intakeTagPanel(trackId);
        if (!live) return;
        if (!writing.current && startedAt === writesDone.current) {
          for (const [key, entry] of wanted.current) {
            if (entry.track === trackId && same(entry.wanted, entry.written)) wanted.current.delete(key);
          }
        }
        setState({ track: trackId, panel, failed: null });
      } catch (error) {
        if (!live) return;
        const failed = messageOf(error);
        setState((shown) => (shown.track === trackId ? { ...shown, failed } : { track: trackId, panel: null, failed }));
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
  }, [trackId]);

  const work = useCallback(async () => {
    if (writing.current) return;
    writing.current = true;
    try {
      for (;;) {
        const entry = [...wanted.current.values()].find((e) => !same(e.wanted, e.written));
        if (!entry) break;
        const { track, dimension } = entry;
        let next: string[];
        let why: string | null;
        if (dimension.edit === "fact") {
          next = [...entry.wanted];
          why = await writersRef.current.setFact(track, dimension.id, next);
        } else {
          // One tick at a time: the first tag whose state differs.
          const tag = dimension.tags.find((t) => entry.wanted.includes(t.id) !== entry.written.includes(t.id));
          if (!tag || tag.myTag === null) { entry.wanted = [...entry.written]; continue; }
          const on = entry.wanted.includes(tag.id);
          next = on ? [...entry.written, tag.id] : entry.written.filter((t) => t !== tag.id);
          why = await writersRef.current.putMyTag(track, tag.myTag, on);
        }
        writesDone.current += 1;
        if (why === null) {
          entry.written = next;
          setFailure(null);
        } else {
          // Not written: back to what is known, and said.
          entry.wanted = [...entry.written];
          setFailure(why);
        }
        rerender();
      }
    } finally {
      writing.current = false;
    }
    refresher.current?.request();
  }, [rerender]);

  const current = state.track === trackId ? state : { panel: null, failed: null };
  const readSet = (dimension: PanelDimension) => current.panel?.track?.set[dimension.id] ?? [];

  const shown = (dimension: PanelDimension) =>
    wanted.current.get(keyOf(trackId, dimension.id))?.wanted ?? readSet(dimension);

  const toggle = (dimension: PanelDimension, tag: string) => {
    if (!current.panel) return;
    const key = keyOf(trackId, dimension.id);
    const entry = wanted.current.get(key) ?? { track: trackId, dimension, wanted: readSet(dimension), written: readSet(dimension) };
    if (dimension.edit === "fact") {
      entry.wanted = toggledTerms(dimension, entry.wanted, tag);
    } else {
      entry.wanted = entry.wanted.includes(tag) ? entry.wanted.filter((t) => t !== tag) : [...entry.wanted, tag];
    }
    wanted.current.set(key, entry);
    rerender();
    void work();
  };

  const retry = useCallback(() => refresher.current?.request(), []);
  return { panel: current.panel, failed: current.failed, failure, shown, toggle, retry };
}
