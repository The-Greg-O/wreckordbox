import { expect, it } from "vitest";
import { createMockBackend } from "./backend-mock";

const tracks = ["100001", "100002", "100003"];
it("locks a selection as one history step and leaves repeated locks alone", async () => {
  const backend = createMockBackend({ trackCount: 4, writable: true });
  let changes = 0;
  backend.onLibraryChanged(() => { changes += 1; });
  await backend.edits.gridLockSelection(tracks, true);
  expect(await Promise.all(tracks.map(async id => (await backend.gridState(id))?.locked))).toEqual([true, true, true]);
  expect(changes).toBe(1);
  await backend.edits.gridLockSelection(tracks, true);
  expect(changes).toBe(1);
  const history = await backend.edits.undoEdit();
  expect(await Promise.all(tracks.map(async id => (await backend.gridState(id))?.locked))).toEqual([false, false, false]);
  expect(history.canUndo).toBe(false);
});
it("preflights the complete selection and refuses protected writes", async () => {
  const backend = createMockBackend({ trackCount: 4, writable: true });
  await expect(backend.edits.gridLockSelection([tracks[0]!, "missing", tracks[2]!], true)).rejects.toThrow();
  expect((await backend.gridState(tracks[0]!))?.locked).toBe(false);
  await backend.setLibraryProtection(true);
  await expect(backend.edits.gridLockSelection(tracks, true)).rejects.toThrow("Library Protection");
  expect(await Promise.all(tracks.map(async id => (await backend.gridState(id))?.locked))).toEqual([false, false, false]);
});
