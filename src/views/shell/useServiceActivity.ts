import { useEffect, useState } from "react";
import { getBackend } from "@/ipc/client";
import type { ServiceActivity } from "@/ipc/serviceActivity";

type Snapshot = ServiceActivity & { statusUnavailable?: boolean };
const POLL_MS = 1000;
const RETRY_MS = 10000;
const TIMEOUT_MS = 5000;

/** Serial reads while visible. A deadline abandons a stuck IPC response;
 * stop events invalidate older snapshots without preventing a later re-host. */
export function useServiceActivity(): Snapshot | null {
  const [activity, setActivity] = useState<Snapshot | null>(null);
  useEffect(() => {
    let live = true;
    let generation = 0;
    let reading = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let deadline: ReturnType<typeof setTimeout> | undefined;
    let unsubscribe: (() => void) | undefined;
    let refresh = () => {};
    const publish = (next: Snapshot) => {
      if (live) setActivity(previous => JSON.stringify(previous) === JSON.stringify(next) ? previous : next);
    };
    const schedule = (delay: number) => {
      clearTimeout(timer);
      if (live && !document.hidden) timer = setTimeout(refresh, delay);
    };
    const visibility = () => {
      clearTimeout(timer);
      if (!document.hidden) refresh();
    };
    document.addEventListener("visibilitychange", visibility);
    void getBackend().then(backend => {
      if (!live) return;
      unsubscribe = backend.onServiceStopped(problem => {
        generation++;
        publish({ connected: false, problem, runs: [] });
        if (!reading) schedule(POLL_MS);
      });
      refresh = () => {
        if (!live || document.hidden || reading) return;
        reading = true;
        const requested = generation;
        let delay = POLL_MS;
        const timeout = new Promise<never>((_, reject) => {
          deadline = setTimeout(() => reject(new Error("Service snapshot timed out")), TIMEOUT_MS);
        });
        void Promise.race([backend.serviceActivity(), timeout]).then(next => {
          if (requested === generation) publish(next);
          if (next.runsUnavailable) delay = RETRY_MS;
        }).catch(() => {
          delay = RETRY_MS;
          if (live && requested === generation) {
            // A failed read cannot prove that a known stopped service recovered.
            setActivity(previous => previous?.problem ? previous : {
              connected: false, problem: null, runs: [], statusUnavailable: true,
            });
          }
        }).finally(() => {
          clearTimeout(deadline);
          reading = false;
          schedule(delay);
        });
      };
      refresh();
    }).catch(() => publish({ connected: false, problem: null, runs: [], statusUnavailable: true }));
    return () => {
      live = false;
      clearTimeout(timer);
      clearTimeout(deadline);
      unsubscribe?.();
      document.removeEventListener("visibilitychange", visibility);
    };
  }, []);
  return activity;
}
