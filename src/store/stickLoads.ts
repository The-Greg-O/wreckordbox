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

/** Every load the window knows of, newest last. */
export function stickLoads(): readonly StickLoad[] {
  return loads;
}

export function useStickLoads(): readonly StickLoad[] {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => loads,
  );
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
  set((all) => [...all, { id, ...stick, state: "confirming", outcome: null, error: null, reportId: operation, kept: false }]);
  let ran = false;
  try {
    await stage(async (load) => {
      ran = true;
      update(id, { state: "running" });
      try {
        const outcome = await load(operation);
        update(id, { state: "done", outcome });
        // Shown here in full: the shell's copy of a load that completed is not kept twice.
        if (outcome.outcome === "loaded") void forget(operation);
      } catch (error) {
        update(id, { state: "failed", error: error instanceof Error ? error.message : String(error) });
      }
    });
  } catch (error) {
    if (!ran) {
      update(id, { state: "failed", error: error instanceof Error ? error.message : String(error) });
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
      })),
    ...all,
  ]);
}

/** For tests: forget every load. */
export function resetStickLoads(): void {
  keptRead = false;
  set(() => []);
}
