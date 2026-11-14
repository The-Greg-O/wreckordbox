/** Match the native selection transaction and its one shared Undo entry. */
export async function lockAnalysisSelection(
  ids: readonly string[], on: boolean,
  port: {
    protected: () => boolean;
    exists: (id: string) => boolean;
    locked: (id: string) => boolean;
    set: (id: string, on: boolean) => void;
    record: (edit: { label: string; undo: () => void; redo: () => void }) => Promise<unknown>;
  },
): Promise<void> {
  if (port.protected()) throw new Error("Library Protection is on.");
  const tracks = [...new Set(ids)];
  if (tracks.some(id => !port.exists(id))) throw new Error("Track no longer exists.");
  const changed = tracks.filter(id => port.locked(id) !== on);
  if (changed.length === 0) return;
  const apply = (expected: boolean, next: boolean) => {
    if (changed.some(id => !port.exists(id) || port.locked(id) !== expected)) {
      throw new Error("The analysis lock has changed since.");
    }
    for (const id of changed) port.set(id, next);
  };
  apply(!on, on);
  await port.record({ label: "Analysis Lock", undo: () => apply(on, !on), redo: () => apply(!on, on) });
}
