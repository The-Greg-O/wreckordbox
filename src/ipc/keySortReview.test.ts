/** @vitest-environment jsdom */
import { expect, it } from "vitest";
import { createMockBackend } from "./backend-mock";

it("sorts the Classic labels for file-tag spellings like the native index", async () => {
  window.history.replaceState(null, "", "/?keyLights=1");
  const backend = createMockBackend();
  window.history.replaceState(null, "", "/");
  const view = await backend.openView({ source: { kind: "collection" }, sort: "key", descending: false, query: "" });
  expect((await backend.fetchRows(view.viewId, 0, 12)).map(row => row.key)).toEqual([
    "Abm", "Am", "Bm", "1d", "c#m", "Dm", "Em", "F#m", "G", "Gm", "Unknown", "",
  ]);
});
