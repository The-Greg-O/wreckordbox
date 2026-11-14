/**
 * The one reader of the Review queue (wbx-74), shared by the Review tab's
 * count and the Review view, so the window reads the queue once however
 * many places show it.
 *
 * It reads when the first reader attaches and again whenever the queue may
 * have changed: `review:changed` (the service's Fact recordings, reattached
 * Facts, Library changes and the DJ's rejections and reopenings from any
 * client, coalesced by the shell), `library:changed` (the window's own
 * edits), `library:ready` (a read that came while the Library loaded), and,
 * while the view shows a Run still working, when the service's active Runs
 * change (`ReviewView`'s `RunsFinishing`), because a Run finishing tells
 * nobody. Reads are coalesced: one in flight, one
 * pending. A failed read is kept as `error` beside the last listing, never
 * hidden behind it.
 */
import { useEffect, useState } from "react";

import { getBackend } from "@/ipc/client";
import type { ReviewListing } from "@/lib/reviewQueue";

export interface ReviewRead {
  listing: ReviewListing | null;
  /** Why the last read failed; `null` when it did not. */
  error: string | null;
  /** When the last read finished (successful or not). */
  at: number | null;
}

type Listener = (read: ReviewRead) => void;

let current: ReviewRead = { listing: null, error: null, at: null };
const listeners = new Set<Listener>();
let running = false;
let pending = false;
/**
 * Which read is the current one. A read that does not settle within
 * `READ_DEADLINE_MS` is given up (said as a failure) and a restart
 * (`restartReviewRead`) begins a new one at once; whatever an older read
 * answers later is ignored.
 */
let generation = 0;
/** Resolved when a read that began after they asked has finished. */
let waitingForNext: (() => void)[] = [];
let waitingForThis: (() => void)[] = [];
let stops: (() => void)[] = [];
let watchers = 0;

/** The longest one read is waited on. */
export const READ_DEADLINE_MS = 10_000;

function tell(next: ReviewRead) {
  current = next;
  for (const listener of listeners) listener(next);
}

function start() {
  generation += 1;
  const mine = generation;
  running = true;
  pending = false;
  waitingForThis = waitingForNext;
  waitingForNext = [];
  let settled = false;
  const settle = (next: ReviewRead) => {
    if (settled || mine !== generation) return;
    settled = true;
    tell(next);
    running = false;
    const done = waitingForThis;
    waitingForThis = [];
    for (const resolve of done) resolve();
    if (pending || waitingForNext.length > 0) start();
  };
  const deadline = setTimeout(() => {
    settle({ ...current, error: `no answer within ${READ_DEADLINE_MS / 1000} s`, at: Date.now() });
  }, READ_DEADLINE_MS);
  void (async () => {
    try {
      const backend = await getBackend();
      const listing = await backend.reviewWaiting();
      settle({ listing, error: null, at: Date.now() });
    } catch (error) {
      settle({ ...current, error: error instanceof Error ? error.message : String(error), at: Date.now() });
    } finally {
      clearTimeout(deadline);
    }
  })();
}

/**
 * Starts a new read now, giving up the one running if there is one (its
 * answer, should it ever come, is ignored); resolves with what the new
 * read found. What Retry runs.
 */
export function restartReviewRead(): Promise<ReviewRead> {
  return new Promise((resolve) => {
    const abandoned = running ? waitingForThis : [];
    waitingForThis = [];
    waitingForNext.push(...abandoned, () => resolve(current));
    running = false;
    start();
  });
}

/** Asks for a read: now if none runs, else once after the one running. */
export function requestReviewRead(): void {
  if (running) pending = true;
  else start();
}

/** Reads again and resolves with what a read begun after this call found. */
export function readReviewNow(): Promise<ReviewRead> {
  return new Promise((resolve) => {
    waitingForNext.push(() => resolve(current));
    if (!running) start();
  });
}

export function reviewRead(): ReviewRead {
  return current;
}

function attach() {
  watchers += 1;
  if (watchers > 1) return;
  void (async () => {
    const backend = await getBackend();
    if (watchers === 0) return;
    stops.push(backend.onReviewChanged(requestReviewRead));
    stops.push(backend.onLibraryChanged(requestReviewRead));
    stops.push(backend.onLibraryReady(requestReviewRead));
    requestReviewRead();
  })();
}

function detach() {
  watchers -= 1;
  if (watchers > 0) return;
  for (const stop of stops) stop();
  stops = [];
  current = { listing: null, error: null, at: null };
}

/** The queue as last read, read while `enabled`. */
export function useReviewRead(enabled: boolean): ReviewRead {
  const [read, setRead] = useState<ReviewRead>(current);
  useEffect(() => {
    if (!enabled) return undefined;
    listeners.add(setRead);
    attach();
    setRead(current);
    return () => {
      listeners.delete(setRead);
      detach();
    };
  }, [enabled]);
  return enabled ? read : { listing: null, error: null, at: null };
}

/** How many Facts wait: the Review tab's count; `null` when unknown. */
export function useReviewCount(enabled: boolean): number | null {
  const read = useReviewRead(enabled);
  return read.error === null && read.listing ? read.listing.facts.length : null;
}
