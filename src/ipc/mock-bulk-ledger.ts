/** Operation effects exposed only by the browser's fixture backend. */
export function createMockBulkLedger() {
  const runs: { id: string; kind: "analysis" | "import"; tracks: string[]; ended: boolean }[] = [];
  if (typeof window !== "undefined") Object.assign(window, { __bulkLedgerRuns: runs });
  return {
    begin: (kind: "analysis" | "import") => {
      const id = crypto.randomUUID();
      runs.push({ id, kind, tracks: [], ended: false });
      return Promise.resolve(id);
    },
    record: (operation: string | undefined, track: string) => {
      const run = runs.find(run => run.id === operation);
      if (run) run.tracks.push(track);
    },
    finish: (operation: string) => {
      const run = runs.find(run => run.id === operation);
      if (!run) return Promise.reject(new Error("Unknown operation"));
      run.ended = true;
      return Promise.resolve(typeof location !== "undefined" && new URLSearchParams(location.search).has("ledgerWarning")
        ? "Library changes were accepted, but their Ledger recording is incomplete."
        : null);
    },
  };
}
