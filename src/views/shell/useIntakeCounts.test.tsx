/** @vitest-environment jsdom */
/* eslint-disable @typescript-eslint/require-await -- Async act flushes effects and their promise continuations. */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import type { IntakeCounts } from "@/lib/intakeQueue";
import { useIntakeCounts } from "./useIntakeCounts";

let changed: () => void = () => undefined;
let ready: () => void = () => undefined;
const read = vi.fn<() => Promise<IntakeCounts>>();
vi.mock("@/ipc/client", () => ({
  getBackend: async () => ({
    intakeCounts: read,
    onLibraryChanged: (listener: () => void) => { changed = listener; return () => undefined; },
    onLibraryReady: (listener: () => void) => { ready = listener; return () => undefined; },
  }),
}));

let root: Root;
let host: HTMLDivElement;
let renders = 0;
let shown: IntakeCounts | null = null;
function Probe() {
  renders += 1;
  shown = useIntakeCounts(true);
  return null;
}

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  renders = 0;
  shown = null;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const flush = async () => { for (let i = 0; i < 5; i += 1) await act(async () => undefined); };

it("reads again when the Library finishes loading, with no edit", async () => {
  read.mockRejectedValueOnce(new Error("The library has not finished loading yet."));
  read.mockResolvedValue({ intake: 7, problems: 2 });
  await act(async () => root.render(<Probe />));
  await flush();
  expect(shown).toBeNull();

  await act(async () => ready());
  await flush();
  expect(shown).toEqual({ intake: 7, problems: 2 });
});

it("does not render again for counts that did not change", async () => {
  read.mockResolvedValue({ intake: 7, problems: 2 });
  await act(async () => root.render(<Probe />));
  await flush();
  const settled = renders;

  for (let i = 0; i < 5; i += 1) {
    await act(async () => changed());
    await flush();
  }
  expect(read.mock.calls.length).toBeGreaterThan(1);
  expect(renders).toBe(settled);

  read.mockResolvedValue({ intake: 6, problems: 2 });
  await act(async () => changed());
  await flush();
  expect(shown).toEqual({ intake: 6, problems: 2 });
  expect(renders).toBe(settled + 1);
});
