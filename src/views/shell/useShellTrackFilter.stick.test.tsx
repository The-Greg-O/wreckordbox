/** @vitest-environment jsdom */
/* eslint-disable @typescript-eslint/require-await -- Async act flushes effects and their promise continuations. */
/**
 * The Track Filter on a Stick list (wbx-45.12): its MyTag choices are the
 * Stick's own, a pick goes by its category's place among the Stick's, picks
 * the Stick does not hold are left out there and kept for the Library, and
 * a Stick whose MyTags cannot be read says so instead of counting.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import type { FilterValues, ViewHandle, ViewSpec } from "@/ipc/types";
import { NO_TRACK, toggleTag } from "@/lib/shellTrackFilter";
import { useShellTrackFilter, type ShellTrackFilter } from "./useShellTrackFilter";

const LIBRARY_TAGS: FilterValues["tags"] = [
  { name: "Situation", tags: ["Peak", "Warm-up"] },
  { name: "Components", tags: ["Synth"] },
];
const STICK_TAGS: FilterValues["tags"] = [
  { name: "Vibe", tags: ["Dark"] },
  { name: "Situation", tags: ["Peak"] },
];
let stickFails: string | null = null;
const opened: ViewSpec[] = [];
const openView = vi.fn(async (spec: ViewSpec): Promise<ViewHandle> => {
  opened.push(spec);
  return { viewId: 1, gen: 1, len: spec.filter ? 2 : 6 };
});
const filterValues = vi.fn(async (spec: ViewSpec): Promise<FilterValues> => {
  if (spec.source.kind !== "device") return { bpms: [], keys: [], tags: LIBRARY_TAGS };
  if (stickFails !== null) throw new Error(stickFails);
  return { bpms: [], keys: [], tags: STICK_TAGS };
});
vi.mock("@/ipc/client", () => ({
  getBackend: async () => ({ openView, filterValues }),
}));

const LIBRARY: ViewSpec = { source: { kind: "collection" }, sort: "trackNo", descending: false, query: "" };
const STICK: ViewSpec = {
  source: { kind: "device", path: "/Volumes/TEST", format: "deviceLibrary", playlist: "1" },
  sort: "trackNo", descending: false, query: "",
};

let root: Root;
let host: HTMLDivElement;
let shown: ShellTrackFilter | null = null;
function Probe({ base }: { base: ViewSpec }) {
  shown = useShellTrackFilter({
    enabled: true, base, query: "", setQuery: () => undefined, generation: 1, tagListRevision: 1, loaded: NO_TRACK,
  });
  return null;
}

beforeEach(() => {
  vi.useFakeTimers();
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  opened.length = 0;
  stickFails = null;
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

async function pickOnTheLibrary() {
  await act(async () => root.render(<Probe base={LIBRARY} />));
  await settle();
  await act(async () => shown!.onChange(toggleTag(shown!.model, "Situation", "Peak")));
  await act(async () => shown!.onChange(toggleTag(shown!.model, "Components", "Synth")));
  await settle();
}

it("on a Stick list the choices are the Stick's and a pick goes by the Stick's category order", async () => {
  await pickOnTheLibrary();
  expect(shown!.viewFilter?.tags).toEqual([
    { category: 0, any: false, tags: ["Peak"] },
    { category: 1, any: false, tags: ["Synth"] },
  ]);

  await act(async () => root.render(<Probe base={STICK} />));
  await settle();
  expect(shown!.categories).toEqual(STICK_TAGS);
  // Situation is second on the Stick; Components is not on it at all.
  expect(shown!.viewFilter?.tags).toEqual([{ category: 1, any: false, tags: ["Peak"] }]);
  expect(opened.at(-1)?.filter?.tags).toEqual([{ category: 1, any: false, tags: ["Peak"] }]);
  expect(shown!.count).toEqual({ state: "known", matching: 2, total: 6 });
  expect(shown!.dropped).toEqual([]);

  // Back on the Library, the pick the Stick lacked is still there.
  await act(async () => root.render(<Probe base={LIBRARY} />));
  await settle();
  expect(shown!.model.myTags).toEqual([
    { category: "Situation", tags: ["Peak"] },
    { category: "Components", tags: ["Synth"] },
  ]);
  expect(shown!.viewFilter?.tags).toEqual([
    { category: 0, any: false, tags: ["Peak"] },
    { category: 1, any: false, tags: ["Synth"] },
  ]);
});

it("a Stick whose MyTags cannot be read says so instead of a count", async () => {
  await pickOnTheLibrary();
  stickFails = "This device has no export.pdb.";
  await act(async () => root.render(<Probe base={STICK} />));
  await settle();
  expect(shown!.count).toEqual({ state: "failed", message: "this Stick's MyTags could not be read: This device has no export.pdb." });
  expect(shown!.viewFilter?.tags).toBeUndefined();
});
