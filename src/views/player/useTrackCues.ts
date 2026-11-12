/**
 * A track's cues, kept current.
 *
 * Fetched when the track changes, and fetched again whenever the backend says
 * the track's cues changed — after an edit from this deck, the other deck,
 * or the hot-cue pads — and after any full reload of the library. One small
 * IPC call each time; the markers on both waveforms and the list beside the
 * deck all draw from this one array, so they move together and nothing
 * flashes.
 *
 * The cues come with the track they were read for and when that read began
 * (wbx-64): after a switch the previous track's cues stay until the new
 * track's read lands, so whatever acts on a cue checks `track` first, and a
 * pad waiting for its write to show waits for a read begun after it. Reads
 * of one track that land out of order keep the newer.
 */
import { useEffect, useRef, useState } from "react";

import type { Cue, RowDto } from "@/ipc/types";
import { getBackend } from "@/ipc/client";

export interface TrackCuesRead {
  cues: Cue[];
  /** The track `cues` were read for, or `null` before any read. */
  track: string | null;
  /** When the read that gave `cues` began (`performance.now()`). */
  readAt: number;
}

const NO_CUES: TrackCuesRead = { cues: [], track: null, readAt: 0 };

export function useTrackCues(
  track: RowDto | null,
  /**
   * Called with the cues of a newly loaded track, once. The deck opens a
   * track on its first memory cue, and that is decided here rather than on
   * every refetch — a cue added mid-track must not move the cue point.
   */
  onLoaded?: (cues: Cue[]) => void,
): TrackCuesRead {
  const [read, setRead] = useState<TrackCuesRead>(NO_CUES);
  // A ref so the fetch effect does not re-run when the callback's identity
  // does; the deck passes a fresh closure each render.
  const loaded = useRef(onLoaded);
  loaded.current = onLoaded;

  useEffect(() => {
    if (!track) {
      setRead(NO_CUES);
      return;
    }
    const id = track.id;
    let live = true;
    let first = true;
    let newest = 0;
    let stopCues: (() => void) | undefined;
    let stopLibrary: (() => void) | undefined;

    const fetch = async () => {
      const readAt = performance.now();
      const backend = await getBackend();
      const found = await backend.trackCues(id);
      // The track may have changed while this was in flight, or a read
      // begun later may have landed first.
      if (!live || readAt < newest) return;
      newest = readAt;
      setRead({ cues: found, track: id, readAt });
      if (first) {
        first = false;
        loaded.current?.(found);
      }
    };
    // Returning from rekordbox is when its cue edits become visible here.
    const onFocus = () => { void fetch(); };
    window.addEventListener("focus", onFocus);

    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      // Subscribed before the first fetch lands, so an edit made in the gap
      // is not missed.
      stopCues = backend.onCuesChanged((changed) => {
        if (changed === id) void fetch();
      });
      stopLibrary = backend.onLibraryChanged(() => {
        void fetch();
      });
      await fetch();
    })();

    return () => {
      live = false;
      window.removeEventListener("focus", onFocus);
      stopCues?.();
      stopLibrary?.();
    };
  }, [track]);

  return read;
}
