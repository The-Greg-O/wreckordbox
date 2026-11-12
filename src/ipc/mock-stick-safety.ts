/**
 * The mock's Stick safety, as the real backend's (wbx-45.4, wbx-45.5).
 *
 * Approval tokens: `stickWritePlan` issues one for the planned operation,
 * bound to the Stick's volume and revision; every Stick-writing call must
 * carry a matching, unused one, or it is refused with the backend's own
 * words and nothing is written. Each write that runs moves the Stick's
 * revision on; replacing the Stick at the same place (`__replaceStick`, or
 * a remount) changes its volume.
 *
 * Logs, for the specs: every Stick write that ran (`__stickWrites`, with
 * its arguments), every refused one (`__stickRefusedWrites`), and every
 * call that in the real backend first runs `rbl_export::recover()`
 * (`__stickRecoveringCalls`); when such a call reaches a Stick marked as
 * holding an unfinished write (`__setStickPending`), the Stick is logged as
 * settled (`__stickRecovered`). A Stick marked with `__setStickSpare`
 * carries the Spare stick marker.
 */
import type { Backend, StickWriteOperation, StickWritePlan, ViewSpec } from "./types";

export interface StickCall {
  method: string;
  path: string;
  /** The write's arguments, as JSON, for the specs to check what was written. */
  args?: unknown;
}

interface Log {
  writes: StickCall[];
  refused: StickCall[];
  recovering: StickCall[];
  recovered: string[];
  pending: Set<string>;
  spare: Set<string>;
  /** Sticks whose next write fails part-way, leaving an unfinished write. */
  failing: Set<string>;
  revision: Map<string, number>;
  volume: Map<string, number>;
  tokens: Map<string, Approval>;
}

type Grant =
  | { exact: string }
  | { exportSome: ReadonlySet<string> }
  | { addTo: { format: string; playlist: string; max: number; duplicates: boolean } };

interface Approval {
  path: string;
  volume: number;
  /** The Stick's revision the next step is bound to; -1 while a step is being written. */
  revision: number;
  steps: Grant[];
}

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

/** Where the log lives without a window (the unit tests). */
const WINDOWLESS: { __stickSafetyLog?: Log } = {};

function log(): Log {
  const holder = (typeof window === "undefined" ? WINDOWLESS : window) as { __stickSafetyLog?: Log };
  if (!holder.__stickSafetyLog) {
    const restored = kept();
    holder.__stickSafetyLog = {
      writes: [], refused: [], recovering: [], recovered: [],
      pending: new Set(restored.pending), spare: new Set(restored.spare), failing: new Set(),
      revision: new Map(), volume: new Map(), tokens: new Map(),
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

/** Another volume is now at `path` (a remount): approvals for the old one no longer hold. */
export function mockStickReplaced(path: string): void {
  const state = log();
  state.volume.set(path, (state.volume.get(path) ?? 0) + 1);
}

const bump = (state: Log, path: string) => { state.revision.set(path, (state.revision.get(path) ?? 0) + 1); };

/** JSON with its keys in order and undefined left out, so equal operations compare equal. */
function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value !== null && typeof value === "object") {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record).sort().filter((k) => record[k] !== undefined)
      .map((k) => `${JSON.stringify(k)}:${canonical(record[k])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

/** One operation as the write call that performs it describes it (the backend's `StickOperation`). */
function key(op: StickWriteOperation): string {
  switch (op.kind) {
    case "exportPlaylist": return canonical({ kind: op.kind, playlist: op.playlist, deleteUnlistedMusic: op.deleteUnlistedMusic === true });
    case "exportTracks": return canonical({ kind: op.kind, tracks: op.tracks });
    case "sync": return canonical({ kind: op.kind, playlists: op.playlists, deleteUnlistedMusic: op.deleteUnlistedMusic === true, automatic: op.automatic === true });
    case "playlistEdit": {
      const edit = op.edit.kind === "add" ? { ...op.edit, duplicates: op.edit.duplicates === true } : op.edit;
      return canonical({ kind: op.kind, format: op.format, edit });
    }
    case "addFiles": return canonical({ kind: op.kind, format: op.format, playlist: op.playlist, files: op.files });
    case "saveSettings": return canonical({ kind: op.kind, settings: op.settings });
    case "format": return canonical({ kind: op.kind, layout: op.layout });
    case "settle":
    case "ensureLibrary":
    case "addToPlaylist":
      return canonical({ kind: op.kind });
  }
}

function grants(op: StickWriteOperation): Grant[] {
  if (op.kind !== "addToPlaylist") return [{ exact: key(op) }];
  return [
    ...(op.exportFirst.length > 0 ? [{ exportSome: new Set(op.exportFirst) }] : []),
    { addTo: { format: op.format, playlist: op.playlist, max: op.count, duplicates: op.duplicates } },
  ];
}

function allows(grant: Grant, op: StickWriteOperation): boolean {
  if ("exact" in grant) return key(op) === grant.exact;
  if ("exportSome" in grant) return op.kind === "exportTracks" && op.tracks.length > 0 && op.tracks.every((t) => grant.exportSome.has(t));
  const want = grant.addTo;
  return op.kind === "playlistEdit" && op.format === want.format && op.edit.kind === "add" && op.edit.playlist === want.playlist &&
    (op.edit.duplicates === true) === want.duplicates && op.edit.tracks.length > 0 && op.edit.tracks.length <= want.max;
}

const REFUSED = "Nothing was written to the Stick. Look at what the write would do again and confirm it.";

/** Uses up `token` for `op` on `path`, as the backend's `consume`: the reason it is refused, or null. */
function consume(state: Log, token: string | undefined | null, op: StickWriteOperation, path: string): string | null {
  if (!token) return `This write to the Stick was not confirmed. ${REFUSED}`;
  const approval = state.tokens.get(token);
  state.tokens.delete(token);
  if (!approval) return `This confirmation was already used, or is unknown. ${REFUSED}`;
  if (approval.path !== path) return `This confirmation was for another Stick. ${REFUSED}`;
  if ((state.volume.get(path) ?? 0) !== approval.volume) return `The Stick at this place is not the one the write was confirmed for. ${REFUSED}`;
  if ((state.revision.get(path) ?? 0) !== approval.revision) return `The Stick changed since the write was confirmed. ${REFUSED}`;
  while (approval.steps.length > 0) {
    const step = approval.steps.shift() as Grant;
    if (allows(step, op)) {
      if (approval.steps.length > 0) state.tokens.set(token, { ...approval, revision: -1 });
      return null;
    }
    if (!("exportSome" in step)) break;
  }
  return `This confirmation was for a different write. ${REFUSED}`;
}

/** After a step of a two-step approval: its next step is bound to the Stick as it is now. */
function advance(state: Log, token: string | undefined | null, path: string) {
  const next = token ? state.tokens.get(token) : undefined;
  if (token && next && next.revision === -1) state.tokens.set(token, { ...next, revision: state.revision.get(path) ?? 0 });
}

/** Each Stick-writing call: where its Stick is, which argument carries the approval, and what it does. */
const WRITES: Record<string, { path: number; approval: number; op: (a: unknown[]) => StickWriteOperation }> = {
  exportPlaylist: { path: 1, approval: 5, op: (a) => ({ kind: "exportPlaylist", playlist: a[0] as string, deleteUnlistedMusic: a[3] === true }) },
  exportTracksToDevice: { path: 1, approval: 4, op: (a) => ({ kind: "exportTracks", tracks: [...(a[0] as string[])] }) },
  devicePlaylistEdit: { path: 0, approval: 3, op: (a) => ({ kind: "playlistEdit", format: a[1] as never, edit: a[2] as never }) },
  devicePlaylistAddFiles: { path: 0, approval: 4, op: (a) => ({ kind: "addFiles", format: a[1] as never, playlist: a[2] as string, files: [...(a[3] as string[])] }) },
  saveDeviceSettings: { path: 0, approval: 2, op: (a) => ({ kind: "saveSettings", settings: a[1] as never }) },
  settleStick: { path: 0, approval: 1, op: () => ({ kind: "settle" }) },
  ensureDeviceLibrary: { path: 0, approval: 2, op: () => ({ kind: "ensureLibrary" }) },
  formatDevice: { path: 0, approval: 2, op: (a) => ({ kind: "format", layout: a[1] as never }) },
};

/** Reads made before a write: they refuse a Stick with an unfinished write. */
const INSPECTIONS = new Set(["devicePlaylistCollectionTracks", "devicePlaylistDuplicates", "devicePlaylistImport", "devicePlaylistImportPlan"]);

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

function installHooks(): void {
  if (typeof window === "undefined") return;
  const hooks = window as unknown as {
    __stickWrites: () => StickCall[];
    __stickRefusedWrites: () => StickCall[];
    __stickRecoveringCalls: () => StickCall[];
    __stickRecovered: () => string[];
    __setStickPending: (path: string, pending: boolean) => void;
    __setStickSpare: (path: string, spare: boolean) => void;
    __failNextStickWrite: (path: string) => void;
    __touchStick: (path: string) => void;
    __replaceStick: (path: string) => void;
  };
  hooks.__stickWrites = () => [...log().writes];
  hooks.__stickRefusedWrites = () => [...log().refused];
  hooks.__stickRecoveringCalls = () => [...log().recovering];
  hooks.__stickRecovered = () => [...log().recovered];
  hooks.__setStickPending = (path, pending) => {
    if (pending) log().pending.add(path);
    else log().pending.delete(path);
    bump(log(), path);
    keep(log());
  };
  hooks.__setStickSpare = (path, spare) => {
    if (spare) log().spare.add(path);
    else log().spare.delete(path);
    keep(log());
  };
  hooks.__failNextStickWrite = (path) => { log().failing.add(path); };
  // Something else changed the Stick (a player, another computer).
  hooks.__touchStick = (path) => { bump(log(), path); };
  hooks.__replaceStick = (path) => { mockStickReplaced(path); };
}

type Call = (...args: unknown[]) => unknown;

/** The mock backend with Stick approvals enforced and every Stick call recorded. */
export function withStickSafetyLog(backend: Backend): Backend {
  installHooks();
  const wrapped = { ...backend } as unknown as Record<string, unknown>;
  const original = (name: string): Call | undefined => {
    const found = (backend as unknown as Record<string, unknown>)[name];
    return typeof found === "function" ? (found as Call).bind(backend) : undefined;
  };

  // Plans issue the tokens.
  const plan = original("stickWritePlan");
  wrapped.stickWritePlan = async (request: { destination: string; operation: StickWriteOperation }) => {
    const found = await (plan?.(request) as Promise<StickWritePlan>);
    // Settling is the one write a Stick with an unfinished write takes.
    if (found.pending && request.operation.kind !== "settle") return { ...found, approval: null };
    const state = log();
    const token = `mock-${Math.random().toString(36).slice(2)}-${Date.now()}`;
    state.tokens.set(token, {
      path: request.destination,
      volume: state.volume.get(request.destination) ?? 0,
      revision: state.revision.get(request.destination) ?? 0,
      steps: grants(request.operation),
    });
    return { ...found, approval: token };
  };

  const settle = (state: Log, path: string) => {
    if (state.pending.has(path)) {
      // The real backend settles the unfinished write before it goes on.
      state.recovered.push(path);
      state.pending.delete(path);
      keep(state);
    }
  };

  for (const [name, at] of Object.entries(WRITES)) {
    const run = original(name);
    if (!run) continue;
    wrapped[name] = (...args: unknown[]) => {
      const state = log();
      const path = args[at.path] as string;
      const token = args[at.approval] as string | undefined;
      // The arguments as the spec reads them back: a few ids and names, not rows.
      const call: StickCall = { method: name, path, args: args.slice(0, at.approval) };
      const refusal = consume(state, token, at.op(args), path);
      if (refusal !== null) {
        state.refused.push(call);
        return Promise.reject(new Error(refusal));
      }
      settle(state, path);
      if (state.failing.delete(path)) {
        state.pending.add(path);
        bump(state, path);
        keep(state);
        return Promise.reject(new Error("The write to the Stick stopped part-way."));
      }
      state.writes.push(call);
      return Promise.resolve(run(...args)).then((result) => {
        bump(state, path);
        advance(state, token, path);
        return result;
      });
    };
  }

  // A sync: each Stick's own approval; a Stick without one is reported and left alone.
  const sync = original("syncDevices");
  wrapped.syncDevices = async (...args: unknown[]) => {
    const state = log();
    const [playlists, destinations, , automatic, , deleteUnlistedMusic, , , approvals] = args as [
      string[], string[], unknown, boolean | undefined, unknown, boolean | undefined, unknown, unknown, Record<string, string> | undefined,
    ];
    const op: StickWriteOperation = { kind: "sync", playlists, deleteUnlistedMusic: deleteUnlistedMusic === true, automatic: automatic === true };
    const approved: string[] = [];
    const refused: { path: string; error: string }[] = [];
    for (const path of destinations) {
      const refusal = consume(state, approvals?.[path], op, path);
      const call: StickCall = { method: "syncDevices", path, args: { playlists, deleteUnlistedMusic: deleteUnlistedMusic === true } };
      if (refusal === null) {
        settle(state, path);
        state.writes.push(call);
        approved.push(path);
      } else {
        state.refused.push(call);
        refused.push({ path, error: refusal });
      }
    }
    const written = approved.length > 0 && sync
      ? await (sync(playlists, approved, ...args.slice(2, 8)) as Promise<{ path: string }[]>)
      : [];
    for (const path of approved) bump(state, path);
    return destinations.map((path) => written.find((r) => r.path === path) ?? refused.find((r) => r.path === path));
  };

  for (const [name, paths] of Object.entries(RECOVERING_READS)) {
    const run = original(name);
    if (!run) continue;
    wrapped[name] = (...args: unknown[]) => {
      const state = log();
      for (const path of paths(args)) {
        state.recovering.push({ method: name, path });
        // The inspections before a write refuse a Stick with an unfinished
        // write rather than read it (wbx-45.4); the rest still settle it
        // (wbx-45.5 replaces them).
        if (INSPECTIONS.has(name) && state.pending.has(path)) {
          return Promise.reject(new Error(
            "A write to this Stick did not finish. Nothing was read from it or written to it; settling the unfinished write is not offered here yet.",
          ));
        }
        settle(state, path);
      }
      return run(...args);
    };
  }
  return wrapped as unknown as Backend;
}
