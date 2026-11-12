import { useEffect, useState } from "react";
import { getBackend } from "@/ipc/client";
import type { ServiceActivity } from "@/ipc/serviceActivity";

/** Only mounted in the shell. Serial polling includes Runs begun before mount
 * and by any client/tool; a slow read never builds up overlapping requests. */
export function useServiceActivity(): ServiceActivity | null {
  const [activity, setActivity] = useState<ServiceActivity | null>(null);
  useEffect(() => {
    let live = true;
    let stopped: string | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let unsubscribe: (() => void) | undefined;
    void getBackend().then((backend) => {
      if (!live) return;
      unsubscribe = backend.onServiceStopped((problem) => {
        stopped = problem;
        if (live) setActivity({ connected: false, problem, runs: [] });
      });
      const refresh = async () => {
        try {
          const next = await backend.serviceActivity();
          // A snapshot requested before the stop must never undo that event.
          if (live && !stopped) setActivity(next);
        } catch {
          if (live && !stopped) setActivity({ connected: false, problem: "Service status unavailable", runs: [] });
        } finally {
          if (live && !stopped) timer = setTimeout(() => { void refresh(); }, 1000);
        }
      };
      void refresh();
    }).catch(() => {
      if (live) setActivity({ connected: false, problem: "Service status unavailable", runs: [] });
    });
    return () => {
      live = false;
      clearTimeout(timer);
      unsubscribe?.();
    };
  }, []);
  return activity;
}
