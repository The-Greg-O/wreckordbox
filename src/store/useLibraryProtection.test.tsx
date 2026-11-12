/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import type { LibraryProtection } from "@/ipc/types";
import { useLibraryProtection, type ProtectionBackend } from "./useLibraryProtection";

/** A promise the test settles when it chooses. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

/**
 * The backend's side: it owns the switch. Every call is held until the
 * test answers it, so the order things arrive in is the test's to choose.
 */
function backend() {
  const calls = {
    adopt: [] as { preference: boolean; answer: ReturnType<typeof deferred<LibraryProtection>> }[],
    read: [] as ReturnType<typeof deferred<LibraryProtection>>[],
    set: [] as boolean[],
  };
  let listener: ((now: LibraryProtection) => void) | null = null;
  const api: ProtectionBackend = {
    adoptLibraryProtection: (preference) => {
      const answer = deferred<LibraryProtection>();
      calls.adopt.push({ preference, answer });
      return answer.promise;
    },
    libraryProtection: () => {
      const answer = deferred<LibraryProtection>();
      calls.read.push(answer);
      return answer.promise;
    },
    setLibraryProtection: (on) => {
      calls.set.push(on);
      return Promise.resolve({ protectLibrary: on, revision: 99 });
    },
    onLibraryProtectionChanged: (heard) => {
      listener = heard;
      return () => {
        listener = null;
      };
    },
  };
  const notify = async (now: LibraryProtection) => {
    await act(async () => {
      listener?.(now);
      await Promise.resolve();
    });
  };
  return { calls, api, notify };
}

let host: HTMLDivElement;
let root: Root;
/** The window's preference, as its interface shows it. */
let preference: boolean;
let rerender: () => Promise<void>;

async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

async function mount(initial: boolean, api: ProtectionBackend) {
  preference = initial;
  const mounted = root;
  // Called from inside the test's own `act` (an answer or a notification).
  const set = (on: boolean) => {
    preference = on;
    mounted.render(<Probe />);
  };
  function Probe() {
    useLibraryProtection(preference, set, () => Promise.resolve(api));
    return null;
  }
  rerender = async () => {
    await act(async () => {
      mounted.render(<Probe />);
      await Promise.resolve();
    });
  };
  await rerender();
  await settle();
}

async function answer<T>(promise: { resolve: (value: T) => void }, value: T) {
  await act(async () => {
    promise.resolve(value);
    await Promise.resolve();
  });
  await settle();
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

it("offers the window's stored preference once and shows what the backend answers", async () => {
  const { calls, api } = backend();
  await mount(false, api);
  expect(calls.adopt.map((call) => call.preference)).toEqual([false]);
  // Nothing recorded and a corrupt file look the same here: the backend says on.
  await answer(calls.adopt[0]!.answer, { protectLibrary: true, revision: 0 });
  expect(preference).toBe(true);
  expect(calls.set).toEqual([]);
});

it("never writes a switch turned elsewhere back to the backend", async () => {
  const { calls, api, notify } = backend();
  await mount(true, api);
  await answer(calls.adopt[0]!.answer, { protectLibrary: true, revision: 3 });

  await notify({ protectLibrary: false, revision: 4 });
  expect(preference).toBe(false);
  await notify({ protectLibrary: true, revision: 5 });
  expect(preference).toBe(true);
  expect(calls.set).toEqual([]);
});

it("ignores a stale notification that arrives after a newer one", async () => {
  const { calls, api, notify } = backend();
  await mount(false, api);
  await answer(calls.adopt[0]!.answer, { protectLibrary: false, revision: 1 });

  await notify({ protectLibrary: true, revision: 3 });
  await notify({ protectLibrary: false, revision: 2 });

  expect(preference).toBe(true);
});

it("ignores a start-up answer that a newer notification overtook", async () => {
  const { calls, api, notify } = backend();
  await mount(false, api);
  // Turned on from the command line while the window's own question was
  // still on its way back.
  await notify({ protectLibrary: true, revision: 6 });
  await answer(calls.adopt[0]!.answer, { protectLibrary: false, revision: 5 });

  expect(preference).toBe(true);
});

it("shows protection on when the backend cannot be asked", async () => {
  const { calls, api } = backend();
  await mount(false, api);
  await act(async () => {
    calls.adopt[0]!.answer.reject(new Error("no backend"));
    await Promise.resolve();
  });
  await settle();
  expect(preference).toBe(true);
  expect(calls.set).toEqual([]);
});

it("puts back a preference changed without the backend, from what the backend says", async () => {
  const { calls, api } = backend();
  await mount(true, api);
  await answer(calls.adopt[0]!.answer, { protectLibrary: true, revision: 2 });

  // Something changed the stored preference behind the backend's back.
  preference = false;
  await rerender();
  expect(calls.read).toHaveLength(1);
  await answer(calls.read[0]!, { protectLibrary: true, revision: 2 });

  expect(preference).toBe(true);
  expect(calls.set).toEqual([]);
});

it("keeps a change Preferences made once the backend recorded it", async () => {
  const { calls, api, notify } = backend();
  await mount(true, api);
  await answer(calls.adopt[0]!.answer, { protectLibrary: true, revision: 2 });

  // Preferences turned it off through the backend; its own window's copy of
  // the preference arrives before the backend's notification does.
  preference = false;
  await rerender();
  await notify({ protectLibrary: false, revision: 3 });
  await answer(calls.read[0]!, { protectLibrary: false, revision: 3 });

  expect(preference).toBe(false);
});
