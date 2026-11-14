/**
 * The window's Stick loads and their reports (wbx-71 review), kept above
 * the Stick view so that a load the DJ confirmed keeps its result whatever
 * the view does meanwhile: switched to Library, another Stick picked, the
 * view gone and back. Each load has an operation id and belongs to one
 * Stick identity (its mount path and volume id, `stickIdentity`): the view
 * shows a report only beside that same Stick, and a Stick replaced at the
 * same path is another identity, so an old report never sits beside it. A
 * report stays until the DJ starts another load on the same Stick or
 * dismisses it; a failure is never dropped by a view change.
 *
 * Each load has an operation id the shell keeps its report under, in the
 * app's state folder (`stick_load_reports`), so a load cut short by closing
 * the window, or one that failed, is reported again at the next start
 * (`readKeptStickLoads`, once per page) until dismissed. A load that completed and
 * was shown here is not kept twice: its kept copy is dismissed at once.
 */
import { useSyncExternalStore } from "react";

import { getBackend } from "@/ipc/client";
import { fromServiceProgress, IDLE, loadStep, type LoadEvent, type LoadMachine, type LoadProgress } from "@/lib/holdToLoad";
import type { KeptLoadReport, LoadOutcome } from "@/lib/stickLoad";

export interface StickLoad {
  /** The operation: one per load the DJ asked for. */
  id: number;
  identity: string;
  path: string;
  name: string;
  /** `confirming`: the hold dialog is open; `running`: the load was confirmed and is writing. */
  state: "confirming" | "running" | "done" | "failed";
  outcome: LoadOutcome | null;
  error: string | null;
  /** The id the shell keeps its report under. */
  reportId: string;
  /** Kept from before the window last closed (not this session's). */
  kept: boolean;
  /** The volume this load's progress events named first; an event naming another is not this load's. */
  volume?: string | null;
  /**
   * The load's state machine (wbx-72, `holdToLoad.ts`): loading (full or
   * shrunk to the status line, with its latest progress), finished, failed.
   */
  machine: LoadMachine;
}

/** One Stick as mounted now: another volume at the same path is another Stick. */
export function stickIdentity(path: string, volumeId: string | null | undefined): string {
  return `${path}\u0000${volumeId ?? ""}`;
}

let loads: readonly StickLoad[] = [];
let next = 1;
const listeners = new Set<() => void>();

function set(change: (all: readonly StickLoad[]) => readonly StickLoad[]): void {
  loads = change(loads);
  for (const listener of listeners) listener();
}

function update(id: number, patch: Partial<StickLoad>): void {
  set((all) => all.map((load) => (load.id === id ? { ...load, ...patch } : load)));
}

/** One step of a load's state machine. */
function step(id: number, event: LoadEvent): void {
  const found = loads.find((load) => load.id === id);
  if (found === undefined) return;
  const machine = loadStep(found.machine, event);
  if (machine !== found.machine) update(id, { machine });
}

/** Listens for every change to the loads (for the store's own hook, and tests). */
export function subscribeStickLoads(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** Every load the window knows of, newest last. */
export function stickLoads(): readonly StickLoad[] {
  return loads;
}

export function useStickLoads(): readonly StickLoad[] {
  return useSyncExternalStore(subscribeStickLoads, () => loads);
}

/**
 * Runs one load: `stage` shows the hold and, once it is held, calls `run`
 * with the load to make; the report lands here whatever happens to the view
 * meanwhile. A hold cancelled or refused before the load ran leaves the
 * Stick's earlier report as it was.
 */
export async function runStickLoad(
  stick: { identity: string; path: string; name: string },
  stage: (run: (load: (operation: string) => Promise<LoadOutcome>) => Promise<void>) => Promise<boolean>,
): Promise<void> {
  const id = next++;
  const operation = operationId();
  listenForProgress();
  // Created once the hold is full: the machine is loading from here.
  set((all) => [...all, {
    id, ...stick, state: "confirming", outcome: null, error: null, reportId: operation, kept: false,
    machine: { state: "loading", shrunk: false, progress: null },
  }]);
  let ran = false;
  try {
    await stage(async (load) => {
      ran = true;
      update(id, { state: "running" });
      try {
        const outcome = await load(operation);
        update(id, { state: "done", outcome });
        step(id, { type: "finished", outcome });
        // Shown here in full: the shell's copy of a load that completed is not kept twice.
        if (outcome.outcome === "loaded") void forget(operation);
      } catch (error) {
        const why = error instanceof Error ? error.message : String(error);
        update(id, { state: "failed", error: why });
        step(id, { type: "failed", error: why });
      }
    });
  } catch (error) {
    if (!ran) {
      const why = error instanceof Error ? error.message : String(error);
      update(id, { state: "failed", error: why });
      step(id, { type: "failed", error: why });
      ran = true;
    }
  }
  if (!ran) {
    // Not held for: nothing happened; the Stick's earlier report stays.
    set((all) => all.filter((load) => load.id !== id));
    return;
  }
  // The newest report for a Stick replaces its older ones of this session.
  set((all) => all.filter((load) => load.id === id || load.kept || load.identity !== stick.identity || load.id > id));
}

function operationId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `op-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

async function forget(reportId: string): Promise<void> {
  try {
    await (await getBackend()).stickLoadReportDismiss(reportId);
  } catch {
    // Still kept: it is shown again at the next start, to be dismissed then.
  }
}

/** The DJ has read the report: gone here and from the shell's kept reports. */
export function dismissStickLoad(id: number): void {
  const found = loads.find((load) => load.id === id);
  set((all) => all.filter((load) => load.id !== id));
  if (found) void forget(found.reportId);
}

let keptRead = false;

/** The reports the shell kept before this session (once per page). */
export async function readKeptStickLoads(): Promise<void> {
  if (keptRead) return;
  keptRead = true;
  let kept: KeptLoadReport[];
  try {
    kept = await (await getBackend()).stickLoadReports();
  } catch {
    keptRead = false;
    return;
  }
  set((all) => [
    ...kept
      .filter((report) => !all.some((load) => load.reportId === report.id))
      .map((report): StickLoad => ({
        id: next++,
        identity: `kept\u0000${report.id}`,
        path: report.stick.path,
        name: report.stick.name,
        state: report.outcome === null ? "failed" : "done",
        outcome: report.outcome,
        error: report.outcome === null ? report.error : null,
        reportId: report.id,
        kept: true,
        machine: report.outcome === null
          ? { state: "failed", outcome: null, error: report.error }
          : loadStep({ state: "loading", shrunk: false, progress: null }, { type: "finished", outcome: report.outcome }),
      })),
    ...all,
  ]);
}

/** For tests: forget every load. */
export function resetStickLoads(): void {
  keptRead = false;
  wanted = null;
  pending.clear();
  if (flushing !== null) clearTimeout(flushing);
  flushing = null;
  set(() => []);
}

/** Shrinks a running load to the status line; it keeps running. */
export function shrinkStickLoad(id: number): void {
  step(id, { type: "shrink" });
}

/** Brings a shrunk load back to fill the Stick view. */
export function restoreStickLoad(id: number): void {
  step(id, { type: "restore" });
}

let wanted: string | null = null;

/**
 * The status line's load, clicked: brought back, and the Stick view asked
 * to show its Stick (`takeWantedStick`), whichever it showed before.
 */
export function showStickLoad(id: number): void {
  const found = loads.find((load) => load.id === id);
  if (found === undefined) return;
  wanted = found.path;
  step(id, { type: "restore" });
  // Told even when the machine did not move (it was not shrunk), so the view picks the Stick.
  for (const listener of listeners) listener();
}

/** The Stick the status line asked the view to show, once. */
export function takeWantedStick(): string | null {
  const path = wanted;
  wanted = null;
  return path;
}

/** How long a burst of progress is gathered before the view hears it. */
const PROGRESS_FLUSH_MS = 100;
/** The latest progress per operation, with the volume it named. */
const pending = new Map<string, { stick: string; volume: string | null; progress: LoadProgress }>();
let flushing: ReturnType<typeof setTimeout> | null = null;

/**
 * One `stick_load_progress` event. A burst is coalesced: the latest for
 * each Stick is kept, and the loads change (and the views draw) at most
 * once per `PROGRESS_FLUSH_MS`. An event is matched on its load's
 * operation id (the window's, which the service echoes), never on the
 * mount path alone: one without an operation, or for another load, a late
 * one from an earlier load among them, changes nothing; so does one naming
 * a volume other than the one this load's events named first, and progress
 * after the load ended.
 */
export function receiveStickLoadProgress(data: unknown): void {
  const event = fromServiceProgress(data);
  if (event === null || event.operation === null) return;
  // Kept per operation and volume: a swap mid-burst is not hidden by the newer event.
  const before = pending.get(event.operation);
  if (before !== undefined && (before.volume !== event.volume || before.stick !== event.stick)) return;
  pending.set(event.operation, { stick: event.stick, volume: event.volume, progress: event.progress });
  flushing ??= setTimeout(flushProgress, PROGRESS_FLUSH_MS);
}

function flushProgress(): void {
  flushing = null;
  const latest = new Map(pending);
  pending.clear();
  let changed = false;
  const next = loads.map((load) => {
    const heard = latest.get(load.reportId);
    if (heard === undefined || load.kept || heard.stick !== load.path) return load;
    const known = load.volume ?? null;
    if (known !== null && heard.volume !== known) return load;
    const machine = loadStep(load.machine, { type: "progress", progress: heard.progress });
    if (machine === load.machine) return load;
    changed = true;
    return { ...load, machine, volume: known ?? heard.volume };
  });
  if (changed) set(() => next);
}

let listening = false;

/** Hears the window's `stick-load:progress` events, once per page. */
function listenForProgress(): void {
  if (listening) return;
  listening = true;
  void getBackend()
    .then((backend) => { backend.onStickLoadProgress(receiveStickLoadProgress); })
    .catch(() => { listening = false; });
}

/** The machine to draw for a Stick: its latest load's, or idle. */
export function machineOf(load: StickLoad | null): LoadMachine {
  return load?.machine ?? IDLE;
}
