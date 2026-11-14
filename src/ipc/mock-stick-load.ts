/**
 * The mock backend's Stick load (wbx-71), answered the way the Library
 * service's `sticks.list`, `sticks.dry_run` and `sticks.load` answer
 * (snake_case, see `stick-load.md` in the notes), from the mock's own
 * Library and a record of what each mock Stick holds.
 *
 * A load is a sync of the chosen playlists: the Stick ends up holding them
 * and no other of the Library's playlists. Stick Protection and the
 * window's hold are enforced by `mock-stick-safety.ts` (`stickViewLoad` is
 * one of its Stick writes); here a load must also carry the plan id of a
 * dry run of exactly this load on this Stick, used once.
 *
 * Hooks on `window`, for the browser tests:
 * - `__stickLoadReads()`: how many times each command was asked
 *   (`{ sticks, dryRuns }`), and `__stickLoadLastDryRun()`: its arguments.
 * - `__setStickFree(path, bytes)`: the Stick's free space from now on.
 * - `__setStickNotLoadable(path, reason | null)`: the service cannot plan
 *   for it (a WAL-mode database): a dry run answers `plan: null` and why.
 * - `__failStickDryRuns(message | null)`: every dry run fails with it.
 * - `__holdStickDryRuns()` / `__releaseStickDryRuns()`: dry runs wait.
 * - `__stickHeld(path)`: what the Stick holds (playlist names, track ids).
 * - `__setStickHolds(path, playlists)`: what it holds, as `[{ id, name, tracks }]`.
 * - `__changeTrackOnLibrary(id, fields)`: the track's data differs from the Stick's.
 * - `__stickLoadPlans()`: every plan id a dry run answered; `__stickLoadCalls()`:
 *   every load that reached the mock service (past protection and the hold).
 * - `__releaseFirstStickDryRun()`: lets only the oldest held dry run answer.
 * - `__failNextStickLoad(path, message)`, `__nextStickLoadIncomplete(path, reason)`:
 *   the next load fails before writing / writes all but one track and one
 *   Native stems set (that one's reason given).
 * - `__holdStickLoads()` / `__releaseStickLoads()`: loads wait to finish.
 * - `__playlistTracks(id)`: a Library playlist's tracks.
 * - `__swapStickAt(path, freeBytes)`: another Stick at the same path (a new
 *   volume id, empty, with that free space); the window sees it on its next
 *   device listing.
 *
 * - Kept reports (the shell's `stick_load_reports`): every load's report
 *   or error, in `sessionStorage["rbl.mock.stickLoadReports"]`, so they
 *   survive a reload as the shell's survive a restart.
 *
 * Each dry run is worked out when it is asked, so an answer held back and
 * let go late is the answer for the Stick as it was then.
 */
import { mockProposeStickWrite, mockStickReplaced } from "./mock-stick-safety";
import type { TreeNode } from "./types";

interface HeldPlaylist { id: string; name: string; tracks: string[] }

interface StickState {
  playlists: HeldPlaylist[];
  lastLoaded: string | null;
  free: number | null;
  notLoadable: string | null;
}

export interface MockStickLoadSource {
  devices: () => readonly { name: string; path: string; freeBytes: number; totalBytes: number; volumeId: string }[];
  tree: () => readonly TreeNode[];
  /** A playlist's tracks, as ids, without changing anything. */
  tracksOf: (playlist: string) => readonly string[];
  titleOf: (track: string) => string;
  spare: (path: string) => boolean;
  protectedNow: (path: string) => boolean;
  pending: (path: string) => boolean;
  wait: <T>(value: T) => Promise<T>;
}

/** What a track takes on a Stick, in the mock. */
const TRACK_BYTES = 10_000_000;
/** A Native stems set, in the mock. */
const STEMS_BYTES = 4_000_000;

const nameOf = (path: string) => path.split("/").filter(Boolean).at(-1) ?? path;

const KEPT = "rbl.mock.stickLoadReports";

function keptReports(): unknown[] {
  try {
    const raw = typeof sessionStorage === "undefined" ? null : sessionStorage.getItem(KEPT);
    return raw === null ? [] : JSON.parse(raw) as unknown[];
  } catch {
    return [];
  }
}

function keepReports(reports: readonly unknown[]): void {
  try {
    sessionStorage.setItem(KEPT, JSON.stringify(reports));
  } catch {
    // No storage: reports last as long as the page.
  }
}

export function createMockStickLoad(source: MockStickLoadSource) {
  const sticks = new Map<string, StickState>();
  const plans = new Map<string, { path: string; playlists: string; revision: number }>();
  const changed = new Map<string, string[]>();
  const reads = { sticks: 0, dryRuns: 0 };
  let lastDryRun: { stick: string; playlists: string[] } | null = null;
  let failDryRuns: string | null = null;
  let holding: (() => void)[] | null = null;
  let loadsHeld: (() => void)[] | null = null;
  const failNext = new Map<string, string>();
  const incompleteNext = new Map<string, string>();
  const issued: string[] = [];
  const loadCalls: { stick: string; playlists: string[]; plan: string }[] = [];
  let revision = 0;

  const stateOf = (path: string): StickState => {
    let found = sticks.get(path);
    if (!found) {
      found = { playlists: [], lastLoaded: null, free: null, notLoadable: null };
      sticks.set(path, found);
    }
    return found;
  };

  const stickJson = (path: string) => {
    const device = source.devices().find((d) => d.path === path);
    const state = stateOf(path);
    const tracks = new Set(state.playlists.flatMap((p) => p.tracks));
    return {
      path,
      name: device?.name ?? nameOf(path),
      kind: source.spare(path) ? "spare" : "gig",
      protected: source.protectedNow(path),
      unfinished_write: source.pending(path),
      last_loaded: state.lastLoaded,
      free_bytes: state.free ?? device?.freeBytes ?? null,
      total_bytes: device?.totalBytes ?? null,
      tracks: state.playlists.length > 0 ? tracks.size : null,
      playlists: state.playlists.length > 0 ? state.playlists.length : null,
    };
  };

  /** The chosen playlists, a folder standing for every playlist in it. */
  const chosenPlaylists = (ids: readonly string[]): HeldPlaylist[] => {
    const tree = source.tree();
    const out: HeldPlaylist[] = [];
    const seen = new Set<string>();
    const add = (node: TreeNode) => {
      if (seen.has(node.id)) return;
      seen.add(node.id);
      out.push({ id: node.id, name: node.name, tracks: [...source.tracksOf(node.id)] });
    };
    for (const id of ids) {
      const at = tree.findIndex((node) => node.id === id);
      const node = tree[at];
      if (!node || (node.kind !== "playlist" && node.kind !== "folder")) {
        throw Object.assign(new Error(`There is no playlist ${id} in the Library.`), { kind: "notFound", detail: "playlist_not_found" });
      }
      if (node.kind === "playlist") add(node);
      else {
        for (let i = at + 1; i < tree.length && (tree[i]?.depth ?? 0) > node.depth; i += 1) {
          const child = tree[i];
          if (child?.kind === "playlist") add(child);
        }
      }
    }
    return out;
  };

  const line = (id: string) => ({ id, title: source.titleOf(id) });

  const dryRunNow = (path: string, ids: readonly string[]) => {
    const state = stateOf(path);
    const chosen = chosenPlaylists(ids);
    const before = new Set(state.playlists.flatMap((p) => p.tracks));
    const after = new Set(chosen.flatMap((p) => p.tracks));
    const added = [...after].filter((id) => !before.has(id));
    const removed = [...before].filter((id) => !after.has(id));
    const kept = [...after].filter((id) => before.has(id));
    const changedTracks = kept.filter((id) => changed.has(id)).map((id) => ({ ...line(id), fields: changed.get(id) ?? [] }));
    const onStick = new Map(state.playlists.map((p) => [p.id, p]));
    const created = chosen.filter((p) => !onStick.has(p.id)).map((p) => p.name);
    const rewritten = chosen.filter((p) => onStick.has(p.id)).map((p) => {
      const old = onStick.get(p.id)?.tracks ?? [];
      const keptOrder = p.tracks.filter((t) => old.includes(t));
      const oldOrder = old.filter((t) => p.tracks.includes(t));
      return {
        name: p.name,
        added: p.tracks.filter((t) => !old.includes(t)).length,
        removed: old.filter((t) => !p.tracks.includes(t)).length,
        reordered: keptOrder.join() !== oldOrder.join(),
      };
    });
    const reordered = rewritten.filter((p) => p.reordered).map((p) => p.name);
    const playlistsRemoved = state.playlists.filter((p) => !chosen.some((c) => c.id === p.id)).map((p) => p.name);
    const stems = source.spare(path) ? added.slice(0, Math.min(added.length, 4)).map(line) : [];
    const free = stickJson(path).free_bytes;
    const needed = added.length * TRACK_BYTES + stems.length * STEMS_BYTES;
    const freed = removed.length * TRACK_BYTES;
    const shortfall = free !== null && needed > free ? needed - free : null;
    revision += 1;
    const plan = state.notLoadable === null ? `mock-load-${revision}` : null;
    if (plan !== null) {
      plans.set(plan, { path, playlists: JSON.stringify(ids), revision: revisionOf(path) });
      // In the store the window's hold approves from, as the hosted service's plans are.
      mockProposeStickWrite(plan, path, { kind: "sync", playlists: [...ids] });
      issued.push(plan);
    }
    return {
      stick: stickJson(path),
      playlists: chosen.map((p) => ({ id: p.id, name: p.name })),
      changes: {
        counts: {
          tracks_added: added.length, tracks_changed: changedTracks.length, audio_changed: 0,
          tracks_removed: removed.length, tracks_missing: 0, audio_reused: kept.length,
          playlists_created: created.length, playlists_rewritten: rewritten.length,
          playlists_reordered: reordered.length, playlists_removed: playlistsRemoved.length, native_stems: stems.length,
        },
        tracks: { added: added.map(line), changed: changedTracks, audio_changed: [], removed: removed.map(line), missing: [] },
        playlists: { created, rewritten, reordered, removed: playlistsRemoved },
        native_stems: stems,
        analysis_written: after.size,
        audio_estimated: true,
      },
      space: {
        free_before: free,
        needed_bytes: needed,
        free_after: free === null || shortfall !== null ? null : free - needed + freed,
        shortfall_bytes: shortfall,
        native_stems_bytes: stems.length * STEMS_BYTES,
        approximate: true,
      },
      plan,
      not_loadable: state.notLoadable,
      chosen,
    };
  };

  /** Moves on whenever the Stick's contents change, so an older plan no longer fits. */
  const revisions = new Map<string, number>();
  const revisionOf = (path: string) => revisions.get(path) ?? 0;
  const touch = (path: string) => { revisions.set(path, revisionOf(path) + 1); };

  if (typeof window !== "undefined") {
    const hooks = window as unknown as Record<string, unknown>;
    hooks.__stickLoadReads = () => ({ ...reads });
    hooks.__stickLoadLastDryRun = () => lastDryRun;
    hooks.__setStickFree = (path: string, bytes: number) => { stateOf(path).free = bytes; };
    hooks.__setStickNotLoadable = (path: string, reason: string | null) => { stateOf(path).notLoadable = reason; };
    hooks.__failStickDryRuns = (message: string | null) => { failDryRuns = message; };
    hooks.__holdStickDryRuns = () => { holding ??= []; };
    hooks.__releaseStickDryRuns = () => {
      const waiting = holding ?? [];
      holding = null;
      for (const go of waiting) go();
    };
    hooks.__stickHeld = (path: string) => {
      const state = stateOf(path);
      return { playlists: state.playlists.map((p) => p.name), tracks: [...new Set(state.playlists.flatMap((p) => p.tracks))] };
    };
    hooks.__setStickHolds = (path: string, playlists: HeldPlaylist[]) => {
      stateOf(path).playlists = playlists.map((p) => ({ ...p, tracks: [...p.tracks] }));
      touch(path);
    };
    hooks.__changeTrackOnLibrary = (id: string, fields: string[]) => { changed.set(id, fields); };
    hooks.__stickLoadPlans = () => [...issued];
    hooks.__stickLoadCalls = () => loadCalls.map((c) => ({ ...c }));
    hooks.__releaseFirstStickDryRun = () => { holding?.shift()?.(); };
    hooks.__failNextStickLoad = (path: string, message: string) => { failNext.set(path, message); };
    hooks.__nextStickLoadIncomplete = (path: string, reason: string) => { incompleteNext.set(path, reason); };
    hooks.__holdStickLoads = () => { loadsHeld ??= []; };
    hooks.__releaseStickLoads = () => {
      const waiting = loadsHeld ?? [];
      loadsHeld = null;
      for (const go of waiting) go();
    };
    hooks.__playlistTracks = (id: string) => [...source.tracksOf(id)];
    hooks.__swapStickAt = (path: string, free: number) => {
      const device = source.devices().find((d) => d.path === path) as { volumeId: string } | undefined;
      if (device) device.volumeId = `${device.volumeId}-swapped-${Date.now()}`;
      sticks.set(path, { playlists: [], lastLoaded: null, free, notLoadable: null });
      touch(path);
      mockStickReplaced(path);
    };
  }

  return {
    stickViewSticks: async (): Promise<unknown> => {
      reads.sticks += 1;
      return source.wait({ sticks: source.devices().map((d) => stickJson(d.path)) });
    },
    stickViewDryRun: async (stick: string, playlists: string[]): Promise<unknown> => {
      reads.dryRuns += 1;
      lastDryRun = { stick, playlists: [...playlists] };
      // Worked out now, for the Stick as it is now; then perhaps held back.
      const answer = (() => {
        if (failDryRuns !== null) return new Error(failDryRuns);
        if (!source.devices().some((d) => d.path === stick)) {
          return Object.assign(new Error(`No Stick is mounted at ${stick}.`), { kind: "notFound", detail: "stick_not_found" });
        }
        if (source.pending(stick)) {
          return Object.assign(new Error("A write to this Stick did not finish; settle it in the window first."), { kind: "unfinishedWrite", detail: "stick_unfinished_write" });
        }
        const { chosen: _chosen, ...planned } = dryRunNow(stick, playlists);
        return planned;
      })();
      if (holding !== null) await new Promise<void>((go) => { holding?.push(go); });
      if (answer instanceof Error) throw answer;
      return source.wait(answer);
    },
    stickLoadReports: (): Promise<unknown[]> => source.wait(keptReports()),
    stickLoadReportDismiss: (id: string): Promise<void> => {
      keepReports(keptReports().filter((r) => (r as { id?: string }).id !== id));
      return source.wait(undefined);
    },
    /**
     * Reached only through `mock-stick-safety.ts`: protected Sticks and loads
     * without the hold never get here. Its report, or its error, is kept as
     * the shell keeps it, under `operation`.
     */
    stickViewLoad: async (stick: string, playlists: string[], plan: string, operation?: string): Promise<unknown> => {
      const id = operation ?? `op-${Date.now()}`;
      const keep = (report: unknown, error: { code: string; message: string } | null) => {
        const device = source.devices().find((d) => d.path === stick);
        keepReports([...keptReports().filter((r) => (r as { id?: string }).id !== id), {
          id, stick: { path: stick, name: device?.name ?? nameOf(stick), volume: device?.volumeId ?? null },
          finished: new Date().toISOString(), report, error,
        }]);
      };
      try {
        const report = await loadNow(stick, playlists, plan) as Record<string, unknown>;
        keep(report, null);
        return { ...report, report_id: id };
      } catch (error) {
        const detail = (error as { detail?: string }).detail ?? "stick_load_failed";
        keep(null, { code: detail, message: error instanceof Error ? error.message : String(error) });
        throw error;
      }
    },
  };

  async function loadNow(stick: string, playlists: string[], plan: string): Promise<unknown> {
      loadCalls.push({ stick, playlists: [...playlists], plan });
      if (loadsHeld !== null) await new Promise<void>((go) => { loadsHeld?.push(go); });
      const failure = failNext.get(stick);
      failNext.delete(stick);
      if (failure !== undefined) throw Object.assign(new Error(failure), { kind: "internal", detail: "stick_load_failed" });
      const bound = plans.get(plan);
      plans.delete(plan);
      if (!bound || bound.path !== stick || bound.playlists !== JSON.stringify(playlists) || bound.revision !== revisionOf(stick)) {
        throw Object.assign(
          new Error("This load has no plan of its own: make a new dry run (the plan is unknown, used, or for another load, or the Stick changed)."),
          { kind: "conflict", detail: "stick_load_unconfirmed" },
        );
      }
      const planned = dryRunNow(stick, playlists);
      if (planned.space.shortfall_bytes !== null) {
        throw Object.assign(new Error("The load does not fit on the Stick."), { kind: "conflict", detail: "stick_too_small" });
      }
      const state = stateOf(stick);
      const before = new Set(state.playlists.flatMap((p) => p.tracks));
      const beforePlaylists = new Map(state.playlists.map((p) => [p.id, p.tracks.join()]));
      const reason = incompleteNext.get(stick) ?? null;
      incompleteNext.delete(stick);
      const all = [...new Set(planned.chosen.flatMap((p) => p.tracks))];
      // Incomplete: the last new track's audio could not be read.
      const added = all.filter((id) => !before.has(id));
      const skipped = reason !== null ? added.at(-1) ?? null : null;
      state.playlists = planned.chosen.map((p) => ({ ...p, tracks: p.tracks.filter((t) => t !== skipped) }));
      state.lastLoaded = new Date().toISOString().replace("T", " ").replace("Z", " +00:00");
      if (state.free !== null) state.free = planned.space.free_after ?? state.free;
      for (const id of all) changed.delete(id);
      touch(stick);
      const writtenTracks = added.filter((id) => id !== skipped).map(line);
      const changedPlaylists = planned.chosen.filter((p) => beforePlaylists.get(p.id) !== p.tracks.join());
      const samePlaylists = planned.chosen.filter((p) => beforePlaylists.get(p.id) === p.tracks.join());
      const stems = planned.changes.native_stems.map((track, at) => (
        reason !== null && at === 0
          ? { track, outcome: "failed", reason, stems_fact: null }
          : { track, outcome: "written", reason: null, stems_fact: at + 1 }
      ));
      return source.wait({
        stick: stickJson(stick),
        outcome: reason === null ? "loaded" : "incomplete",
        error: reason === null ? null : { code: "stick_load_incomplete", message: "Not everything planned is on the Stick." },
        planned: planned.changes,
        written: { tracks: writtenTracks, playlists: changedPlaylists.map((p) => p.name) },
        not_written: { tracks: skipped === null ? [] : [line(skipped)], playlists: [] },
        unchanged: { tracks: all.filter((id) => before.has(id)).map(line), playlists: samePlaylists.map((p) => p.name) },
        native_stems: stems,
      });
  }
}
