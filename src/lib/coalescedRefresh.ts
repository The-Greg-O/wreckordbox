/**
 * A refresh that never piles up (wbx-62 review): at most one read in
 * flight and one pending. However many times it is asked while a read
 * runs (a burst of `library:changed` while an agent's Run edits), it reads
 * exactly once more after that read, and that read sees all of it. Each
 * read holds the hosted service, so this bounds the work a burst can cost.
 */
export interface Refresher {
  /** Asks for a read: now if none runs, else once after the one running. */
  request(): void;
  /** Starts nothing more, the pending read included. */
  stop(): void;
}

export function coalescedRefresh(run: () => Promise<void>): Refresher {
  let running = false;
  let pending = false;
  let stopped = false;
  const start = () => {
    running = true;
    pending = false;
    void run()
      .catch(() => undefined)
      .finally(() => {
        running = false;
        if (pending && !stopped) start();
      });
  };
  return {
    request() {
      if (stopped) return;
      if (running) pending = true;
      else start();
    },
    stop() {
      stopped = true;
      pending = false;
    },
  };
}
