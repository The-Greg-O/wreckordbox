/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ServiceActivity } from "@/ipc/serviceActivity";
import { StatusLine } from "./StatusLine";
import { Localization } from "@/i18n";

let stop: (message: string) => void;
const unsubscribe = vi.fn();
const read = vi.fn<() => Promise<ServiceActivity>>();
vi.mock("@/ipc/client", () => ({
  getBackend: async () => ({
    serviceActivity: read,
    onServiceStopped: (listener: typeof stop) => { stop = listener; return unsubscribe; },
  }),
}));

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  vi.clearAllMocks();
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

const connected = { connected: true, problem: null, runs: [] };

it("keeps the current connection text when the translation catalog loads", async () => {
  read.mockResolvedValue(connected);
  await act(async () => root.render(<Localization><StatusLine serviceProblem={null} /></Localization>));
  expect(host.textContent).toContain("Service connected");
});

it("does not replace a stop with an older connected snapshot, or hide it behind another error", async () => {
  let resolve!: (value: ServiceActivity) => void;
  read.mockReturnValue(new Promise(done => { resolve = done; }));
  await act(async () => root.render(<StatusLine serviceProblem={null} error="Could not play the track" />));
  expect(host.textContent).toContain("Connecting to service");
  act(() => stop("The Library service has stopped"));
  await act(async () => resolve({ ...connected, runs: [{ id: 1, description: "Stale work" }] }));
  expect(host.textContent).toContain("The Library service has stopped");
  expect(host.textContent).toContain("Could not play the track");
  expect(host.textContent).not.toContain("Service connected");
  expect(host.textContent).not.toContain("Stale work");
  await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
  expect(read).toHaveBeenCalledTimes(1);
});

it("clears stale work on a failed read and recovers on the next read", async () => {
  read.mockResolvedValueOnce({ ...connected, runs: [{ id: 1, description: "Tag the backlog" }] })
    .mockRejectedValueOnce(new Error("IPC unavailable"))
    .mockResolvedValue(connected);
  await act(async () => root.render(<StatusLine serviceProblem={null} />));
  expect(host.textContent).toContain("Tag the backlog");
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(host.textContent).toContain("Service status unavailable");
  expect(host.textContent).not.toContain("Tag the backlog");
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(host.textContent).toContain("Service connected");
});

it("never overlaps a slow read and cleans up when the shell goes away", async () => {
  let resolve!: (value: ServiceActivity) => void;
  read.mockReturnValue(new Promise(done => { resolve = done; }));
  await act(async () => root.render(<StatusLine serviceProblem={null} />));
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(read).toHaveBeenCalledTimes(1);
  act(() => root.render(null));
  expect(unsubscribe).toHaveBeenCalledOnce();
  await act(async () => resolve(connected));
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(read).toHaveBeenCalledTimes(1);
});

it("keeps the existing analysis Stop and log actions available", async () => {
  read.mockResolvedValue(connected);
  const cancel = vi.fn();
  const showLog = vi.fn();
  await act(async () => root.render(<StatusLine serviceProblem={null} onCancelAnalysis={cancel}
    analysisProgress={{ completed: 2, total: 5 }} error="Analysis failed" onShowLog={showLog} />));
  const buttons = [...host.querySelectorAll("button")];
  act(() => buttons.find(button => button.textContent === "Stop")?.click());
  act(() => buttons.find(button => button.textContent === "Analysis failed")?.click());
  expect(cancel).toHaveBeenCalledOnce();
  expect(showLog).toHaveBeenCalledOnce();
  expect(host.textContent).toContain("Analyzing 2/5 tracks");
});
