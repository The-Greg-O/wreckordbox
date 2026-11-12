/** @vitest-environment jsdom */
/* eslint-disable @typescript-eslint/require-await -- Async act flushes effects and their promise continuations. */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import type { FilterValues, ViewHandle, ViewSpec } from "@/ipc/types";
import { NO_TRACK, setEnergyRange, toggleTag } from "@/lib/shellTrackFilter";
import { useShellTrackFilter, type ShellTrackFilter } from "./useShellTrackFilter";

/** The Tag List the backend holds: what a list of it opens to. */
let tagList = 5;
let tags: FilterValues["tags"] = [{ name: "Situation", tags: ["Peak", "Warm-up"] }];
const openView = vi.fn(async (spec: ViewSpec): Promise<ViewHandle> => ({
  viewId: 1, gen: 1, len: spec.filter ? Math.max(0, tagList - 2) : tagList,
}));
const filterValues = vi.fn(async (): Promise<FilterValues> => ({ bpms: [], keys: [], tags }));
vi.mock("@/ipc/client", () => ({
  getBackend: async () => ({ openView, filterValues }),
}));

const TAG_LIST: ViewSpec = { source: { kind: "tagList" }, sort: "trackNo", descending: false, query: "" };

let root: Root;
let host: HTMLDivElement;
let shown: ShellTrackFilter | null = null;
function Probe({ revision }: { revision: number }) {
  shown = useShellTrackFilter({
    enabled: true, base: TAG_LIST, query: "", setQuery: () => undefined, generation: 1, tagListRevision: revision, loaded: NO_TRACK,
  });
  return null;
}

beforeEach(() => {
  vi.useFakeTimers();
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  tagList = 5;
  tags = [{ name: "Situation", tags: ["Peak", "Warm-up"] }];
  shown = null;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

const settle = async () => {
  for (let i = 0; i < 5; i += 1) {
    await act(async () => { vi.advanceTimersByTime(200); });
  }
};

it("the count follows a Tag List change, as the list itself does", async () => {
  await act(async () => root.render(<Probe revision={1} />));
  await settle();
  await act(async () => shown!.onChange(setEnergyRange(shown!.model, 3, 5)));
  await settle();
  expect(shown!.count).toEqual({ state: "known", matching: 3, total: 5 });

  // A track leaves the Tag List: no Library generation, only its revision.
  tagList = 4;
  await act(async () => root.render(<Probe revision={2} />));
  await settle();
  expect(shown!.count).toEqual({ state: "known", matching: 2, total: 4 });
});

it("a picked tag the Library no longer holds is dropped from the filter and named", async () => {
  await act(async () => root.render(<Probe revision={1} />));
  await settle();
  await act(async () => shown!.onChange(toggleTag(toggleTag(shown!.model, "Situation", "Peak"), "Situation", "Warm-up")));
  await settle();
  expect(shown!.viewFilter?.tags).toEqual([{ category: 0, any: false, tags: ["Peak", "Warm-up"] }]);

  // Warm-up is deleted elsewhere; the Library's tags are read again.
  tags = [{ name: "Situation", tags: ["Peak"] }];
  await act(async () => root.render(<Probe revision={2} />));
  await settle();
  expect(shown!.model.myTags).toEqual([{ category: "Situation", tags: ["Peak"] }]);
  expect(shown!.viewFilter?.tags).toEqual([{ category: 0, any: false, tags: ["Peak"] }]);
  expect(shown!.dropped).toEqual(["Situation: Warm-up"]);
});

it("typing does not read the choices on every keystroke", async () => {
  function Typing({ query }: { query: string }) {
    shown = useShellTrackFilter({
      enabled: true, base: TAG_LIST, query, setQuery: () => undefined, generation: 1, tagListRevision: 1, loaded: NO_TRACK,
    });
    return null;
  }
  await act(async () => root.render(<Typing query="" />));
  await settle();
  const before = filterValues.mock.calls.length;
  for (const query of ["p", "pe", "pea", "peak"]) {
    await act(async () => root.render(<Typing query={query} />));
    await act(async () => { vi.advanceTimersByTime(30); });
  }
  await settle();
  // One read of the list's choices for the burst (the Library's are not re-read on typing).
  expect(filterValues.mock.calls.length - before).toBe(1);
});
