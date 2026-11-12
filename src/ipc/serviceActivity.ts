/** The service's read-only snapshot for the status line. */
export interface ActiveRun {
  id: number;
  description: string;
}

export interface ServiceActivity {
  connected: boolean;
  problem: string | null;
  runs: ActiveRun[];
}
