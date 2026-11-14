// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createMockBackend } from "@/ipc/backend-mock";
import type { Backend } from "@/ipc/types";
import { PREFERENCES_KEY } from "@/lib/preferences";
import { App } from "./App";

const held = vi.hoisted(() => ({
  backend: null as Backend | null,
  lock: null as ((ids: readonly string[], on: boolean) => void) | null,
}));
vi.mock("@/ipc/client", async (importOriginal) => {
  const original = await importOriginal<typeof import("@/ipc/client")>();
  const { createMockBackend } = await import("@/ipc/backend-mock");
  held.backend = createMockBackend({ trackCount: 3 });
  return { ...original, getBackend: () => Promise.resolve(held.backend) };
});
vi.mock("@/views/player/Player", () => ({ Player: () => null }));
vi.mock("@/views/browser/TrackTable", () => ({
  TrackTable: (props: { onAnalysisLock: (ids: readonly string[], on: boolean) => void }) => {
    held.lock = props.onAnalysisLock;
    return null;
  },
}));
vi.mock("@/views/statusbar/StatusBar", () => ({
  StatusBar: ({ activity, error }: { activity: string; error: string | null }) =>
    <div><span data-testid="activity">{activity}</span><span role="alert">{error}</span></div>,
}));
let host: HTMLDivElement;
let root: Root;
beforeEach(async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  localStorage.setItem(PREFERENCES_KEY, JSON.stringify({ advanced: { protectLibrary: false } }));
  held.backend = createMockBackend({ trackCount: 3 });
  held.lock = null;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  await act(async () => { root.render(<App />); await Promise.resolve(); });
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 0)); });
});
afterEach(() => {
  act(() => root.unmount());
  host.remove(); localStorage.clear(); vi.unstubAllGlobals(); vi.restoreAllMocks();
});
it("submits the selected tracks once, so the window owns one Undo operation", async () => {
  const batch = vi.spyOn(held.backend!.edits, "gridLockSelection").mockResolvedValue();
  const one = vi.spyOn(held.backend!.edits, "gridLock");
  await act(async () => { held.lock!(["1", "2", "3"], true); await Promise.resolve(); });
  expect(batch).toHaveBeenCalledExactlyOnceWith(["1", "2", "3"], true);
  expect(one).not.toHaveBeenCalled();
  expect(host.querySelector('[data-testid="activity"]')?.textContent).toContain("Locked the analysis of 3 tracks.");
});
it("shows a refused batch without reporting that the selection was locked", async () => {
  vi.spyOn(held.backend!.edits, "gridLockSelection").mockRejectedValue(new Error("Library Protection is on."));
  await act(async () => { held.lock!(["1", "2", "3"], true); await Promise.resolve(); });
  expect(host.querySelector('[role="alert"]')?.textContent).toBe("Library Protection is on.");
  expect(host.querySelector('[data-testid="activity"]')?.textContent).not.toContain("Locked the analysis");
});
