/**
 * The deck's cue and grid writes, per track (wbx-64 review 3).
 *
 * Intake's Mark Clean, P and Skip wait for every write the track still has
 * on its way, as they wait for its tag writes (`TagStore.settled`), and do
 * not move on when one failed. A failure stays with the track it came from,
 * whichever track is loaded when it arrives, until a later write of the same
 * kind to that track lands or the DJ dismisses it.
 *
 * `track` takes a promise that answers the failure's message, or `null`
 * when the write landed; it never rejects.
 */
export type TrackWriteKind = "cues" | "grid";

export interface TrackWriteFailure {
  track: string;
  kind: TrackWriteKind;
  message: string;
}

export class TrackWrites {
  private readonly onTheirWay = new Map<string, Set<Promise<unknown>>>();
  private readonly failed = new Map<string, Map<TrackWriteKind, string>>();
  private readonly listeners = new Set<() => void>();

  /** Follows one write of `track`, from now until `outcome` answers. */
  track(track: string, kind: TrackWriteKind, outcome: Promise<string | null>): void {
    const set = this.onTheirWay.get(track) ?? new Set<Promise<unknown>>();
    this.onTheirWay.set(track, set);
    const done = outcome
      .catch((error: unknown) => (error instanceof Error ? error.message : String(error)))
      .then((message) => {
        const kinds = this.failed.get(track) ?? new Map<TrackWriteKind, string>();
        if (message === null) kinds.delete(kind);
        else kinds.set(kind, message);
        if (kinds.size === 0) this.failed.delete(track);
        else this.failed.set(track, kinds);
      })
      .finally(() => {
        set.delete(done);
        if (set.size === 0) this.onTheirWay.delete(track);
        this.tell();
      });
    set.add(done);
    this.tell();
  }

  /** Whether `track` has a write on its way. */
  pending(track: string): boolean {
    return (this.onTheirWay.get(track)?.size ?? 0) > 0;
  }

  /**
   * Waits until `track` has no write on its way, including writes begun
   * while waiting; answers why one failed (`"grid: …"`), or `null`.
   */
  async settled(track: string): Promise<string | null> {
    for (;;) {
      const set = this.onTheirWay.get(track);
      if (!set || set.size === 0) break;
      await Promise.all([...set]);
    }
    const kinds = this.failed.get(track);
    const first = kinds ? [...kinds.entries()][0] : undefined;
    return first ? `${first[0]}: ${first[1]}` : null;
  }

  /** Every failure still standing, oldest track first. */
  failures(): TrackWriteFailure[] {
    return [...this.failed.entries()].flatMap(([track, kinds]) =>
      [...kinds.entries()].map(([kind, message]) => ({ track, kind, message })));
  }

  dismiss(track: string, kind: TrackWriteKind): void {
    const kinds = this.failed.get(track);
    if (!kinds?.delete(kind)) return;
    if (kinds.size === 0) this.failed.delete(track);
    this.tell();
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  private tell(): void {
    for (const listener of this.listeners) listener();
  }
}

/** The window's one registry: the deck writes, Intake waits. */
export const deckWrites = new TrackWrites();
