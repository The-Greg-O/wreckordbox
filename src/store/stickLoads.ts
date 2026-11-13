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
 */
import { useSyncExternalStore } from "react";

import type { LoadOutcome } from "@/lib/stickLoad";

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
  stage: (run: (load: () => Promise<LoadOutcome>) => Promise<void>) => Promise<boolean>,
): Promise<void> {
  const id = next++;
  set((all) => [...all, { id, ...stick, state: "confirming", outcome: null, error: null }]);
  let ran = false;
  try {
    await stage(async (load) => {
      ran = true;
      update(id, { state: "running" });
      try {
        update(id, { state: "done", outcome: await load() });
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
  // The newest report for a Stick replaces its older ones.
  set((all) => all.filter((load) => load.id === id || load.identity !== stick.identity || load.id > id));
}

/** The DJ has read the report. */
export function dismissStickLoad(id: number): void {
  set((all) => all.filter((load) => load.id !== id));
}

/** For tests: forget every load. */
export function resetStickLoads(): void {
  set(() => []);
}
