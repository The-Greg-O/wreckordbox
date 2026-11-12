/**
 * The Intake tag panel's state and writes (wbx-65), kept by the Intake view
 * so it outlives the panel of any one track.
 *
 * Reading: one panel per track, read on request, coalesced (one read in
 * flight and one pending per track). A read that fails keeps what was
 * shown and says so. A read that began before a write of that track
 * finished is not trusted: it is read again.
 *
 * Writing: a click sets what the DJ wants a dimension of a track to hold
 * (`toggle`), shown at once. One worker writes, one write at a time, each
 * the whole dimension (`intake.set_tags`), from the version of the panel
 * the DJ clicked: clicks made while a write of that dimension is on its
 * way become one write. A click is always for the track it was made on.
 *
 * Failures are kept per track and dimension until the DJ acknowledges or
 * retries them; an unrelated success never clears one, a later success of
 * the same dimension does. A refused write drops what was asked for that
 * dimension (it is not shown as made); a stale one also reads the panel
 * again. A Fact the service could not record is a failure of its own kind.
 * `settled(track)` waits for every write of a track and answers why any of
 * them failed, so "Mark Clean and next" can wait and stop.
 */
import { toggledTerms, type PanelDimension, type TagPanel, type Unmapped } from "./tagPanel";

export interface TagWrite {
  track: string;
  dimension: string;
  terms: string[];
  version: string;
  dropUnmapped: string[];
}

export interface TagBackend {
  read(track: string): Promise<TagPanel>;
  write(write: TagWrite): Promise<{ panel: TagPanel; factsNotRecorded: string[] }>;
}

export interface TagFailure {
  key: string;
  track: string;
  dimension: string;
  /** The dimension's label when it failed. */
  label: string;
  message: string;
  /** `failed`: nothing was written. `not_recorded`: written, but the DJ's Fact was not recorded. */
  kind: "failed" | "not_recorded";
  /** What the DJ asked for, for Retry. */
  terms: string[];
  dropUnmapped: string[];
}

interface Entry {
  track: string;
  dimension: PanelDimension;
  wanted: string[];
  drop: string[];
  written: string[];
  version: string;
}

interface Shown {
  panel: TagPanel | null;
  failed: string | null;
  readAt: Date | null;
}

const same = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((x, i) => x === b[i]);
const keyOf = (track: string, dimension: string) => `${track}\n${dimension}`;

/** The message and code of a rejected command, whatever shape it came in. */
export function tagError(error: unknown): { message: string; code: string } {
  const object = (typeof error === "object" && error !== null ? error : {}) as { message?: unknown; detail?: unknown; code?: unknown };
  const message = typeof object.message === "string" ? object.message : String(error);
  const code = typeof object.code === "string" ? object.code : typeof object.detail === "string" ? object.detail : "";
  return { message, code };
}

export class TagStore {
  private readonly backend: TagBackend;
  private readonly panels = new Map<string, Shown>();
  private readonly reads = new Map<string, { running: boolean; pending: boolean }>();
  /** Writes finished per track: a read begun before one is read again. */
  private readonly writesDone = new Map<string, number>();
  private readonly entries = new Map<string, Entry>();
  private readonly failed = new Map<string, TagFailure>();
  private writing: string | null = null;
  /** Tracks whose Retry is reading the panel before it writes. */
  private readonly retrying = new Map<string, number>();
  private working = false;
  private waiters: (() => void)[] = [];
  private readonly listeners = new Set<() => void>();
  private revision = 0;

  constructor(backend: TagBackend) {
    this.backend = backend;
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  getRevision = (): number => this.revision;

  private changed() {
    this.revision += 1;
    for (const listener of [...this.listeners]) listener();
  }

  panel(track: string): Shown {
    return this.panels.get(track) ?? { panel: null, failed: null, readAt: null };
  }

  /** Reads `track`'s panel: now, or once more after the read running. */
  refresh(track: string): void {
    const state = this.reads.get(track) ?? { running: false, pending: false };
    this.reads.set(track, state);
    if (state.running) { state.pending = true; return; }
    state.running = true;
    state.pending = false;
    const startedAt = this.writesDone.get(track) ?? 0;
    void this.backend.read(track).then(
      (panel) => {
        if ((this.writesDone.get(track) ?? 0) !== startedAt || this.writing === track) {
          state.pending = true;
        } else {
          this.panels.set(track, { panel, failed: null, readAt: new Date() });
          this.forget(track, panel);
        }
      },
      (error: unknown) => {
        const shown = this.panel(track);
        this.panels.set(track, { ...shown, failed: tagError(error).message });
      },
    ).finally(() => {
      state.running = false;
      this.changed();
      if (state.pending) this.refresh(track);
    });
  }

  /** What was asked of a track and is now what the panel says needs no showing of its own. */
  private forget(track: string, panel: TagPanel) {
    for (const [key, entry] of this.entries) {
      if (entry.track !== track || entry.drop.length > 0) continue;
      const read = panel.track?.set[entry.dimension.id] ?? [];
      if (same(entry.wanted, entry.written) && same(read, entry.written)) this.entries.delete(key);
    }
  }

  /** The tags shown for `dimension` on `track`: what the DJ asked for, else what was read. */
  shown(track: string, dimension: PanelDimension): string[] {
    return this.entries.get(keyOf(track, dimension.id))?.wanted ?? this.panel(track).panel?.track?.set[dimension.id] ?? [];
  }

  /** The values no tag stands for, less those the DJ has asked to take off. */
  unmapped(track: string, dimension: PanelDimension): Unmapped[] {
    const dropping = this.entries.get(keyOf(track, dimension.id))?.drop ?? [];
    return (this.panel(track).panel?.track?.unmapped[dimension.id] ?? []).filter((value) => !dropping.includes(value.id));
  }

  private entryFor(track: string, dimension: PanelDimension): Entry | null {
    const key = keyOf(track, dimension.id);
    const existing = this.entries.get(key);
    if (existing) return existing;
    const panel = this.panel(track).panel;
    const version = panel?.track?.versions[dimension.id];
    if (!panel || version === undefined) return null;
    const read = panel.track?.set[dimension.id] ?? [];
    const entry = { track, dimension, wanted: [...read], drop: [], written: [...read], version };
    this.entries.set(key, entry);
    return entry;
  }

  /** A click on `tag` of `dimension` on `track`. */
  toggle(track: string, dimension: PanelDimension, tag: string): void {
    const entry = this.entryFor(track, dimension);
    if (!entry) return;
    entry.wanted = toggledTerms(dimension, entry.wanted, tag);
    this.changed();
    void this.work();
  }

  /** Takes off a value no tag stands for: only ever by the DJ's own click. */
  dropUnmapped(track: string, dimension: PanelDimension, value: string): void {
    const entry = this.entryFor(track, dimension);
    if (!entry || entry.drop.includes(value)) return;
    entry.drop = [...entry.drop, value];
    this.changed();
    void this.work();
  }

  /** Whether anything of `track` is waiting to be written. */
  pending(track: string): boolean {
    if (this.writing === track || (this.retrying.get(track) ?? 0) > 0) return true;
    for (const entry of this.entries.values()) {
      if (entry.track === track && (!same(entry.wanted, entry.written) || entry.drop.length > 0)) return true;
    }
    return false;
  }

  /** Waits for every write of `track`; answers why any failed, or `null`. */
  async settled(track: string): Promise<string | null> {
    while (this.pending(track)) {
      await new Promise<void>((resolve) => this.waiters.push(resolve));
    }
    const failures = this.failures().filter((failure) => failure.track === track && failure.kind === "failed");
    return failures.length === 0 ? null : failures.map((failure) => `${failure.label}: ${failure.message}`).join("; ");
  }

  failures(): TagFailure[] {
    return [...this.failed.values()];
  }

  acknowledge(key: string): void {
    if (this.failed.delete(key)) this.changed();
  }

  /** Writes again what a failed write asked for, from the panel as it is now. */
  retry(key: string): void {
    const failure = this.failed.get(key);
    if (!failure) return;
    this.failed.delete(key);
    this.retrying.set(failure.track, (this.retrying.get(failure.track) ?? 0) + 1);
    this.changed();
    const done = () => {
      this.retrying.set(failure.track, (this.retrying.get(failure.track) ?? 1) - 1);
      this.wake();
    };
    void (async () => {
      let panel: TagPanel;
      try {
        panel = await this.backend.read(failure.track);
      } catch (error) {
        this.failed.set(key, { ...failure, message: tagError(error).message });
        this.changed();
        return;
      }
      this.panels.set(failure.track, { panel, failed: null, readAt: new Date() });
      const dimension = panel.dimensions.find((d) => d.id === failure.dimension);
      const version = panel.track?.versions[failure.dimension];
      if (!dimension || version === undefined) {
        this.failed.set(key, { ...failure, message: "That dimension is no longer in the panel." });
        this.changed();
        return;
      }
      const read = panel.track?.set[failure.dimension] ?? [];
      const terms = failure.terms.filter((term) => dimension.tags.some((tag) => tag.id === term));
      this.entries.set(key, { track: failure.track, dimension, wanted: terms, drop: [...failure.dropUnmapped], written: [...read], version });
      this.changed();
      void this.work();
    })().finally(done);
  }

  private async work(): Promise<void> {
    if (this.working) return;
    this.working = true;
    try {
      for (;;) {
        const next = [...this.entries.entries()].find(([, e]) => !same(e.wanted, e.written) || e.drop.length > 0);
        if (!next) break;
        const [key, entry] = next;
        const write: TagWrite = {
          track: entry.track,
          dimension: entry.dimension.id,
          terms: [...entry.wanted],
          version: entry.version,
          dropUnmapped: [...entry.drop],
        };
        this.writing = entry.track;
        try {
          const answer = await this.backend.write(write);
          const written = answer.panel.track?.set[write.dimension] ?? write.terms;
          const version = answer.panel.track?.versions[write.dimension] ?? entry.version;
          entry.written = [...written];
          entry.version = version;
          entry.drop = entry.drop.filter((value) => !write.dropUnmapped.includes(value));
          this.panels.set(entry.track, { panel: answer.panel, failed: null, readAt: new Date() });
          this.failed.delete(key);
          if (answer.factsNotRecorded.length > 0) {
            this.failed.set(key, {
              key, track: write.track, dimension: write.dimension, label: entry.dimension.label,
              message: answer.factsNotRecorded.join("; "), kind: "not_recorded", terms: write.terms, dropUnmapped: [],
            });
          }
          if (same(entry.wanted, entry.written) && entry.drop.length === 0) this.entries.delete(key);
        } catch (error) {
          const { message, code } = tagError(error);
          this.entries.delete(key);
          this.failed.set(key, {
            key, track: write.track, dimension: write.dimension, label: entry.dimension.label,
            message: code === "tag_panel_stale" ? `${message} (read again)` : message,
            kind: "failed", terms: [...entry.wanted], dropUnmapped: [...entry.drop],
          });
          if (code === "tag_panel_stale") this.writesDone.set(entry.track, (this.writesDone.get(entry.track) ?? 0) + 1);
        } finally {
          this.writesDone.set(entry.track, (this.writesDone.get(entry.track) ?? 0) + 1);
          this.writing = null;
          this.changed();
          this.wake();
        }
        if (!this.entries.has(key)) this.refresh(entry.track);
      }
    } finally {
      this.working = false;
      this.wake();
    }
  }

  /** Lets every `settled` check again. */
  private wake() {
    const waiting = this.waiters;
    this.waiters = [];
    for (const wake of waiting) wake();
  }
}
