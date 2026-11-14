import type { ActiveRun, ServiceActivity } from "./serviceActivity";

/** Controllable service work, with no audio processing or Library writes. */
/** `extraRuns`: Runs other mock parts hold open (a split's, wbx-67). */
export function mockServiceActivity(extraRuns: () => ActiveRun[] = () => []) {
  let runs: ActiveRun[] = [];
  let problem: string | null = null;
  let runsUnavailable = false;
  let hung = false;
  const listeners = new Set<(message: string) => void>();
  if (typeof window !== "undefined") {
    const hooks = window as unknown as {
      __setActiveRuns: (active: ActiveRun[]) => void;
      __stopService: (message: string) => void;
      __setRunsUnavailable: (value: boolean) => void;
      __hangServiceReads: (value: boolean) => void;
      __rehostService: () => void;
      __initialActiveRuns?: ActiveRun[];
      __initialServiceProblem?: string;
    };
    hooks.__setRunsUnavailable = value => { runsUnavailable = value; };
    hooks.__hangServiceReads = value => { hung = value; };
    hooks.__rehostService = () => { problem = null; hung = false; };
    runs = hooks.__initialActiveRuns ?? [];
    problem = hooks.__initialServiceProblem ?? null;
    hooks.__setActiveRuns = (active) => { runs = active; };
    hooks.__stopService = (message) => {
      problem = message;
      for (const listener of listeners) listener(message);
    };
  }
  return {
    serviceActivity: (): Promise<ServiceActivity> => {
      const hooks = window as unknown as { __serviceActivityReads?: number };
      hooks.__serviceActivityReads = (hooks.__serviceActivityReads ?? 0) + 1;
      return hung ? new Promise(() => {}) : Promise.resolve({ connected: problem === null, problem, runs: problem || runsUnavailable ? [] : [...runs, ...extraRuns()], runsUnavailable });
    },
    serviceProblem: () => Promise.resolve(problem),
    onServiceStopped: (listener: (message: string) => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}
