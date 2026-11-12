/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { useLibraryProtection, type ProtectionBackend } from "./useLibraryProtection";

/** The backend's side of the switch: what is recorded for the service. */
function backend(recorded: boolean | null) {
  const state = {
    recorded,
    writes: [] as boolean[],
    listener: null as ((on: boolean) => void) | null,
  };
  const api: ProtectionBackend = {
    libraryProtection: () => Promise.resolve(state.recorded),
    recordLibraryProtection: (on) => {
      state.recorded = on;
      state.writes.push(on);
      return Promise.resolve();
    },
    onLibraryProtectionChanged: (listener) => {
      state.listener = listener;
      return () => {
        state.listener = null;
      };
    },
  };
  return { state, api };
}

let host: HTMLDivElement;
let root: Root;
/** The window's own preference, as its interface holds it. */
let preference: boolean;
let rerender: () => Promise<void>;

async function mount(initial: boolean, api: ProtectionBackend) {
  preference = initial;
  const set = (on: boolean) => {
    preference = on;
    void rerender();
  };
  function Probe() {
    useLibraryProtection(preference, set, () => Promise.resolve(api));
    return null;
  }
  rerender = async () => {
    await act(async () => {
      root.render(<Probe />);
      await Promise.resolve();
    });
  };
  await rerender();
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

it("records the window's preference for the service when nothing is recorded yet", async () => {
  const { state, api } = backend(null);
  await mount(false, api);
  expect(state.recorded).toBe(false);
  expect(preference).toBe(false);
});

it("takes up a switch turned from the command line while the window was closed", async () => {
  const { state, api } = backend(false);
  await mount(true, api);
  expect(preference).toBe(false);
  // The window's stale preference never overwrote what was recorded.
  expect(state.writes).not.toContain(true);
  expect(state.recorded).toBe(false);
});

it("takes up protection turned on from the command line without ever recording it off", async () => {
  const { state, api } = backend(true);
  await mount(false, api);
  expect(preference).toBe(true);
  expect(state.writes).not.toContain(false);
});

it("records a change the DJ makes in Preferences", async () => {
  const { state, api } = backend(true);
  await mount(true, api);
  preference = false;
  await rerender();
  expect(state.recorded).toBe(false);
});

it("takes up a switch turned from the command line while the window is open", async () => {
  const { state, api } = backend(true);
  await mount(true, api);
  await act(async () => {
    state.recorded = false;
    state.listener?.(false);
    await Promise.resolve();
  });
  expect(preference).toBe(false);
  expect(state.recorded).toBe(false);
});
