/**
 * What the mock does to a Stick, recorded for the browser specs that must
 * prove the window writes nothing to a Stick and settles nothing on one
 * without the DJ asking (wbx-45.4, wbx-45.5).
 *
 * Every backend call that writes to a Stick is logged in `__stickWrites`.
 * Every call that, in the real backend, first runs `rbl_export::recover()`
 * (it reads a Stick through `device::read` or writes to it) is logged in
 * `__stickRecoveringCalls`; when the Stick has an unfinished write
 * (`__setStickPending`), such a call is what would settle it, and the path
 * is logged in `__stickRecovered` too. The read-only outline is not such a
 * call. A Stick marked with `__setStickSpare` carries the Spare stick marker.
 */
import type { Backend, ViewSpec } from "./types";

export interface StickCall {
  method: string;
  path: string;
}

interface Log {
  writes: StickCall[];
  recovering: StickCall[];
  recovered: string[];
  pending: Set<string>;
  spare: Set<string>;
  /** Sticks whose next write fails part-way, leaving an unfinished write. */
  failing: Set<string>;
}

/** Calls that write to a Stick, and where each finds the Stick's path. */
const WRITES: Record<string, (args: unknown[]) => readonly string[]> = {
  exportPlaylist: (a) => [a[1] as string],
  exportTracksToDevice: (a) => [a[1] as string],
  syncDevices: (a) => a[1] as string[],
  devicePlaylistEdit: (a) => [a[0] as string],
  devicePlaylistAddFiles: (a) => [a[0] as string],
  formatDevice: (a) => [a[0] as string],
};

/** Calls that only read, but through the real backend's recovering path. */
const RECOVERING_READS: Record<string, (args: unknown[]) => readonly string[]> = {
  deviceLibraries: (a) => [a[0] as string],
  devicePlaylistCollectionTracks: (a) => [a[0] as string],
  devicePlaylistDuplicates: (a) => [a[0] as string],
  devicePlaylistImport: (a) => [a[0] as string],
  devicePlaylistImportPlan: (a) => [a[0] as string],
  openView: (a) => {
    const source = (a[0] as ViewSpec | undefined)?.source;
    return source?.kind === "device" ? [source.path] : [];
  },
};

/** Kept across a reload, as a Stick's state survives the window restarting. */
const KEPT = "rbl.mock.stickState";

function kept(): { pending: string[]; spare: string[] } {
  try {
    const raw = typeof sessionStorage === "undefined" ? null : sessionStorage.getItem(KEPT);
    return raw === null ? { pending: [], spare: [] } : JSON.parse(raw) as { pending: string[]; spare: string[] };
  } catch {
    return { pending: [], spare: [] };
  }
}

function keep(state: Log): void {
  try {
    sessionStorage.setItem(KEPT, JSON.stringify({ pending: [...state.pending], spare: [...state.spare] }));
  } catch {
    // No storage: the state lasts until the page goes.
  }
}

function log(): Log {
  const holder = (typeof window === "undefined" ? {} : window) as { __stickSafetyLog?: Log };
  if (!holder.__stickSafetyLog) {
    const restored = kept();
    holder.__stickSafetyLog = {
      writes: [], recovering: [], recovered: [],
      pending: new Set(restored.pending), spare: new Set(restored.spare), failing: new Set(),
    };
  }
  return holder.__stickSafetyLog;
}

/** Whether the mock's Stick at `path` has an unfinished write. */
export function mockStickPending(path: string): boolean {
  return log().pending.has(path);
}

/** Whether the mock's Stick at `path` carries the Spare stick marker. */
export function mockStickSpare(path: string): boolean {
  return log().spare.has(path);
}

function installHooks(): void {
  if (typeof window === "undefined") return;
  const hooks = window as unknown as {
    __stickWrites: () => StickCall[];
    __stickRecoveringCalls: () => StickCall[];
    __stickRecovered: () => string[];
    __setStickPending: (path: string, pending: boolean) => void;
    __setStickSpare: (path: string, spare: boolean) => void;
    __failNextStickWrite: (path: string) => void;
  };
  hooks.__stickWrites = () => [...log().writes];
  hooks.__stickRecoveringCalls = () => [...log().recovering];
  hooks.__stickRecovered = () => [...log().recovered];
  hooks.__setStickPending = (path, pending) => {
    if (pending) log().pending.add(path);
    else log().pending.delete(path);
    keep(log());
  };
  hooks.__failNextStickWrite = (path) => { log().failing.add(path); };
  hooks.__setStickSpare = (path, spare) => {
    if (spare) log().spare.add(path);
    else log().spare.delete(path);
    keep(log());
  };
}

/** The mock backend with every Stick write and recovering call recorded. */
export function withStickSafetyLog(backend: Backend): Backend {
  installHooks();
  const wrapped = { ...backend } as Record<string, unknown>;
  const wrap = (name: string, paths: (args: unknown[]) => readonly string[], write: boolean) => {
    const original = wrapped[name];
    if (typeof original !== "function") return;
    wrapped[name] = (...args: unknown[]) => {
      const state = log();
      for (const path of paths(args)) {
        (write ? state.writes : state.recovering).push({ method: name, path });
        if (state.pending.has(path)) {
          // The real backend settles the unfinished write before it goes on.
          state.recovered.push(path);
          state.pending.delete(path);
          keep(state);
        }
        if (write && state.failing.delete(path)) {
          state.pending.add(path);
          keep(state);
          return Promise.reject(new Error("The write to the Stick stopped part-way."));
        }
      }
      return (original as (...a: unknown[]) => unknown).apply(backend, args);
    };
  };
  for (const [name, paths] of Object.entries(WRITES)) wrap(name, paths, true);
  for (const [name, paths] of Object.entries(RECOVERING_READS)) wrap(name, paths, false);
  return wrapped as unknown as Backend;
}
