/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";

import { onDemand } from "./onDemand";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

/** A load the test settles by hand, as a slow or failing chunk would. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

function Hello({ name }: { name: string }) {
  return <p data-testid="hello">Hello {name}</p>;
}

it("draws the view once its code has arrived, with the props it was given", async () => {
  const load = deferred<typeof Hello>();
  const view = onDemand("Greeting", () => load.promise);
  act(() => root.render(<view.View name="DJ" />));
  expect(host.querySelector('[data-testid="hello"]')).toBeNull();
  await act(async () => { load.resolve(Hello); await load.promise; });
  expect(host.querySelector('[data-testid="hello"]')?.textContent).toBe("Hello DJ");
});

it("draws at once, with no waiting step, when its code was loaded ahead", async () => {
  const view = onDemand("Greeting", () => Promise.resolve(Hello));
  await act(() => view.preload());
  act(() => root.render(<view.View name="DJ" />));
  expect(host.querySelector('[data-testid="hello"]')?.textContent).toBe("Hello DJ");
});

it("shows a load that failed as a failure, and Retry loads it again", async () => {
  let attempts = 0;
  const view = onDemand("Intake", () => {
    attempts += 1;
    return attempts === 1 ? Promise.reject(new Error("chunk missing")) : Promise.resolve(Hello);
  });
  await act(async () => { await view.preload().catch(() => {}); });
  act(() => root.render(<view.View name="DJ" />));
  const alert = host.querySelector('[role="alert"]');
  expect(alert?.textContent).toContain("Intake");
  expect(alert?.textContent).toContain("chunk missing");
  expect(host.querySelector('[data-testid="hello"]')).toBeNull();

  const retry = [...host.querySelectorAll("button")].find((b) => b.textContent === "Retry");
  expect(retry?.closest("[data-own-activation]")).not.toBeNull();
  await act(async () => { retry!.click(); await Promise.resolve(); await Promise.resolve(); });
  expect(host.querySelector('[data-testid="hello"]')?.textContent).toBe("Hello DJ");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("keeps keys pressed while its code is on the way and hands them to the view when it arrives", async () => {
  const heard: string[] = [];
  function Listener() {
    // Like the Intake view: its keys are heard on window, in the capture phase.
    const ref = (el: HTMLElement | null) => {
      if (!el) return;
      const on = (event: KeyboardEvent) => { heard.push(event.key); };
      window.addEventListener("keydown", on, true);
    };
    return <div ref={ref} data-testid="listener" />;
  }
  const load = deferred<typeof Listener>();
  const view = onDemand("Intake", () => load.promise);
  act(() => root.render(<view.View />));
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "c", bubbles: true }));
  // A key something earlier already took (the view keys 1 to 4) is not kept.
  const taken = new KeyboardEvent("keydown", { key: "2", bubbles: true, cancelable: true });
  taken.preventDefault();
  window.dispatchEvent(taken);
  expect(heard).toEqual([]);
  await act(async () => { load.resolve(Listener); await load.promise; });
  await act(async () => { await new Promise((r) => setTimeout(r, 0)); });
  expect(heard).toEqual(["c"]);
});
