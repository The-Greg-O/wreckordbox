import type { ActiveRun, ServiceActivity } from "./serviceActivity";

/** Controllable service work, with no audio processing or Library writes. */
export function mockServiceActivity() {
  let runs: ActiveRun[] = [];
  let problem: string | null = null;
  const listeners = new Set<(message: string) => void>();
  if (typeof window !== "undefined") {
    const hooks = window as unknown as {
      __setActiveRuns: (active: ActiveRun[]) => void;
      __stopService: (message: string) => void;
      __initialActiveRuns?: ActiveRun[];
      __initialServiceProblem?: string;
    };
    runs = hooks.__initialActiveRuns ?? [];
    problem = hooks.__initialServiceProblem ?? null;
    hooks.__setActiveRuns = (active) => { runs = active; };
    hooks.__stopService = (message) => {
      problem = message;
      for (const listener of listeners) listener(message);
    };
  }
  return {
    serviceActivity: (): Promise<ServiceActivity> => Promise.resolve({ connected: problem === null, problem, runs: problem ? [] : runs }),
    serviceProblem: () => Promise.resolve(problem),
    onServiceStopped: (listener: (message: string) => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}
