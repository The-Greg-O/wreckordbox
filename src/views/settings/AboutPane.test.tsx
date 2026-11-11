/**
 * @vitest-environment jsdom
 *
 * About says what this is, which version, what it is based on and under
 * what terms, and nothing in it leaves the machine: no update check, no
 * donation and no links.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { __setBackend } from "@/ipc/client";
import type { Backend } from "@/ipc/types";
import { AboutPane } from "./AboutPane";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

let host: HTMLDivElement;
let root: Root;

const mount = () =>
  act(async () => {
    root.render(<AboutPane />);
    await Promise.resolve();
    await Promise.resolve();
  });

beforeEach(() => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  __setBackend({ appVersion: () => Promise.resolve("0.4.0") } as unknown as Backend);
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  __setBackend(null);
});

describe("AboutPane", () => {
  it("names the app and its version", async () => {
    await mount();
    expect(host.querySelector("h1")?.textContent).toBe("wreckordbox");
    expect(host.querySelector('[data-testid="about-version"]')?.textContent).toBe("v0.4.0");
  });

  it("credits rbxport and keeps its copyright in plain text", async () => {
    await mount();
    expect(host.textContent).toContain("wreckordbox is based on rbxport.");
    expect(host.textContent).toContain("Copyright © Chris Le");
    expect(host.textContent).toContain("GNU General Public License version 2");
  });

  it("offers nothing to press and names no address", async () => {
    await mount();
    expect(host.querySelectorAll("button, a, input, select")).toHaveLength(0);
    expect(host.textContent).not.toMatch(/https?:|update|Support|TRIODE/i);
  });
});
