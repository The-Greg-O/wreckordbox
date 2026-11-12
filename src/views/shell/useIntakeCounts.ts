/**
 * How many tracks wait in Intake and in Problems (wbx-62), for the Intake
 * tab and the tree. Read from the Library service when the shell is on, and
 * again after every change to the Library, whoever made it (the window, the
 * CLI, an agent: the service's change reaches the window as
 * `library:changed`), and when the Library finishes loading
 * (`library:ready`), so a first read that came too early is retried with no
 * edit needed.
 *
 * Reads are coalesced (`coalescedRefresh`): one in flight and one pending,
 * so a burst of changes costs at most one more read. An answer equal to the
 * one shown keeps the same object, so nothing downstream re-renders. A read
 * that fails shows no count rather than a stale one.
 */
import { useEffect, useRef, useState } from "react";

import { getBackend } from "@/ipc/client";
import { coalescedRefresh } from "@/lib/coalescedRefresh";
import type { IntakeCounts } from "@/lib/intakeQueue";

function same(a: IntakeCounts | null, b: IntakeCounts | null): boolean {
  return a === b || (a !== null && b !== null && a.intake === b.intake && a.problems === b.problems);
}

export function useIntakeCounts(enabled: boolean): IntakeCounts | null {
  const [counts, setCounts] = useState<IntakeCounts | null>(null);
  // What is shown, so an equal answer does not even ask React to render.
  const current = useRef<IntakeCounts | null>(null);
  useEffect(() => {
    if (!enabled) {
      current.current = null;
      setCounts(null);
      return undefined;
    }
    let live = true;
    const stops: (() => void)[] = [];
    const show = (next: IntakeCounts | null) => {
      if (!live || same(current.current, next)) return;
      current.current = next;
      setCounts(next);
    };
    const refresh = coalescedRefresh(async () => {
      const backend = await getBackend();
      try {
        show(await backend.intakeCounts());
      } catch {
        // Not loaded yet, or no service: no count. `library:ready` or the
        // next change reads again.
        show(null);
      }
    });
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      // Listen first, then read, so nothing between the two is missed.
      stops.push(backend.onLibraryChanged(() => refresh.request()));
      stops.push(backend.onLibraryReady(() => refresh.request()));
      refresh.request();
    })();
    return () => {
      live = false;
      refresh.stop();
      for (const stop of stops) stop();
    };
  }, [enabled]);
  return counts;
}
