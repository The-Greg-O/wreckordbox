/**
 * How many tracks wait in Intake and in Problems (wbx-62), for the Intake
 * tab and the tree. Read from the Library service when the shell is on, and
 * again after every change to the Library, whoever made it (the window, the
 * CLI, an agent): the service's change reaches the window as
 * `library:changed`. An answer that arrives after a later one is dropped.
 */
import { useEffect, useState } from "react";

import { getBackend } from "@/ipc/client";
import type { IntakeCounts } from "@/lib/intakeQueue";

export function useIntakeCounts(enabled: boolean): IntakeCounts | null {
  const [counts, setCounts] = useState<IntakeCounts | null>(null);
  useEffect(() => {
    if (!enabled) {
      setCounts(null);
      return undefined;
    }
    let live = true;
    let asked = 0;
    let stop: (() => void) | undefined;
    const read = async () => {
      const ask = ++asked;
      try {
        const backend = await getBackend();
        const answer = await backend.intakeCounts();
        if (live && ask === asked) setCounts(answer);
      } catch {
        // No service (the window cannot write, or it is starting): no count
        // rather than a stale one. The view itself says why.
        if (live && ask === asked) setCounts(null);
      }
    };
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      // Listen first, then read, so a change between the two is not missed.
      stop = backend.onLibraryChanged(() => { void read(); });
      void read();
    })();
    return () => {
      live = false;
      stop?.();
    };
  }, [enabled]);
  return counts;
}
