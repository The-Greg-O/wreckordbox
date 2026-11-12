/** @vitest-environment jsdom */
/* eslint-disable @typescript-eslint/require-await -- Async act flushes effects and their promise continuations. */
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
  Object.defineProperty(document, "hidden", { configurable: true, value: false });
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
  await act(async () => root.render(<Localization><StatusLine /></Localization>));
  expect(host.textContent).toContain("Service connected");
});

it("does not replace a stop with an older connected snapshot, or hide it behind another error", async () => {
  let resolve!: (value: ServiceActivity) => void;
  read.mockReturnValue(new Promise(done => { resolve = done; }));
  await act(async () => root.render(<StatusLine error="Could not play the track" />));
  expect(host.textContent).toContain("Connecting to service");
  act(() => stop("The Library service has stopped"));
  await act(async () => resolve({ ...connected, runs: [{ id: 1, description: "Stale work" }] }));
  expect(host.textContent).toContain("The Library service has stopped");
  expect(host.textContent).toContain("Could not play the track");
  expect(host.textContent).not.toContain("Service connected");
  expect(host.textContent).not.toContain("Stale work");
});

it("clears stale work on a failed read and recovers on the next read", async () => {
  read.mockResolvedValueOnce({ ...connected, runs: [{ id: 1, description: "Tag the backlog" }] })
    .mockRejectedValueOnce(new Error("IPC unavailable"))
    .mockResolvedValue(connected);
  await act(async () => root.render(<StatusLine />));
  expect(host.textContent).toContain("Tag the backlog");
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(host.textContent).toContain("Service status unavailable");
  expect(host.textContent).not.toContain("Tag the backlog");
  await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
  expect(host.textContent).toContain("Service connected");
});

it("announces only connection and Runs politely, and errors as alerts", async () => {
  read.mockResolvedValue(connected);
  await act(async () => root.render(<StatusLine selection="30 Tracks"
    analysisProgress={{ completed: 2, total: 5 }} error="Could not play" />));
  const status = host.querySelector('[role="status"]')!;
  expect(status.textContent).toBe("Service connected");
  expect(status.querySelector("button")).toBeNull();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("Could not play");
  expect(host.querySelector("footer")).toBeNull();
  act(() => stop("The Library service has stopped"));
  expect([...host.querySelectorAll('[role="alert"]')].some(el => el.textContent?.includes("has stopped"))).toBe(true);
});

it("keeps the service connected when Runs cannot be read and backs off", async () => {
  read.mockResolvedValueOnce({ ...connected, runsUnavailable: true }).mockResolvedValue(connected);
  await act(async () => root.render(<StatusLine />));
  expect(host.textContent).toContain("Service connected");
  expect(host.textContent).toContain("Runs unavailable");
  expect(host.querySelector('[role="alert"]')).toBeNull();
  await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
  expect(read).toHaveBeenCalledTimes(1);
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(host.textContent).not.toContain("Runs unavailable");
});

it("recovers after a stop and subsequent successful re-host", async () => {
  read.mockResolvedValue(connected);
  await act(async () => root.render(<StatusLine />));
  act(() => stop("Another window owns the Library"));
  expect(host.textContent).toContain("Another window");
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(host.textContent).toContain("Service connected");
  expect(host.textContent).not.toContain("Another window");
});

it("pauses hidden reads and refreshes when visible", async () => {
  read.mockResolvedValue(connected);
  Object.defineProperty(document, "hidden", { configurable: true, value: true });
  await act(async () => root.render(<StatusLine />));
  await act(async () => { await vi.advanceTimersByTimeAsync(30000); });
  expect(read).not.toHaveBeenCalled();
  await act(async () => {
    Object.defineProperty(document, "hidden", { configurable: true, value: false });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  expect(read).toHaveBeenCalledOnce();
  await act(async () => {
    Object.defineProperty(document, "hidden", { configurable: true, value: true });
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(10000);
  });
  expect(read).toHaveBeenCalledOnce();
});

it("times out a hung read, backs off, and ignores its late response", async () => {
  let late!: (value: ServiceActivity) => void;
  read.mockResolvedValueOnce({ ...connected, runs: [{ id: 1, description: "Old work" }] })
    .mockImplementationOnce(() => new Promise(resolve => { late = resolve; }))
    .mockResolvedValue(connected);
  await act(async () => root.render(<StatusLine />));
  await act(async () => { await vi.advanceTimersByTimeAsync(6000); });
  expect(host.textContent).toContain("Service status unavailable");
  expect(host.textContent).not.toContain("Old work");
  await act(async () => { await vi.advanceTimersByTimeAsync(10000); });
  expect(host.textContent).toContain("Service connected");
  await act(async () => late({ ...connected, runs: [{ id: 1, description: "Old work" }] }));
  expect(host.textContent).not.toContain("Old work");
});

it("keeps the version and the Stick load stage", async () => {
  read.mockResolvedValue(connected);
  await act(async () => root.render(<StatusLine version="1.2.3" exports={[
    { path: "/tmp/stick", name: "Spare", state: "ejecting", done: 0, total: 0, title: "" },
  ]} />));
  expect(host.textContent).toContain("1.2.3");
  expect(host.textContent).toContain("Safely ejecting");
});

it("never overlaps a slow read and cleans up when the shell goes away", async () => {
  let resolve!: (value: ServiceActivity) => void;
  read.mockReturnValue(new Promise(done => { resolve = done; }));
  await act(async () => root.render(<StatusLine />));
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
  await act(async () => root.render(<StatusLine onCancelAnalysis={cancel}
    analysisProgress={{ completed: 2, total: 5 }} error="Analysis failed" onShowLog={showLog} />));
  const buttons = [...host.querySelectorAll("button")];
  act(() => buttons.find(button => button.textContent === "Stop")?.click());
  act(() => buttons.find(button => button.textContent === "Analysis failed")?.click());
  expect(cancel).toHaveBeenCalledOnce();
  expect(showLog).toHaveBeenCalledOnce();
  expect(host.textContent).toContain("Analyzing 2/5 tracks");
});
