/**
 * @vitest-environment jsdom
 *
 * The rendering half of the sleeve: what it draws before it has an image, and
 * that it asks again rather than leaving the square empty for good.
 *
 * jsdom does not fetch an `<img>`, which is what makes this testable at all —
 * `load` and `error` are dispatched here, so both paths run to their end
 * instead of waiting on a network.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ARTWORK_RETRIES } from "@/ipc/artwork";

declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}

let host: HTMLDivElement;
let root: Root;
/** Imported after the Tauri marker is set: the module reads it once, at load. */
let Artwork: typeof import("./Artwork").Artwork;

beforeEach(async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  // Tauri's own `convertFileSrc`, as it answers on macOS and Linux.
  (window as unknown as Record<string, unknown>)["__TAURI_INTERNALS__"] = {
    convertFileSrc: (path: string, protocol: string) => `${protocol}://localhost/${encodeURIComponent(path)}`,
  };
  vi.resetModules();
  ({ Artwork } = await import("./Artwork"));
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  vi.useRealTimers();
});

const image = () => host.querySelector("img");

function mount(trackId = "7") {
  act(() => root.render(<Artwork trackId={trackId} />));
}

describe("Artwork", () => {
  it("draws nothing until the image has actually decoded", () => {
    mount();
    // Present so it can load, and not shown: a broken-file glyph in the square
    // is what this exists to prevent.
    expect(image()?.getAttribute("data-state")).toBe("pending");
    expect(image()?.getAttribute("alt")).toBe("");
  });

  it("asks with CORS where the Linux webview refuses a plain image", () => {
    // #44: under `pnpm dev` the page is `http://localhost:1420`, and WebKitGTK
    // 2.54.0 and 2.54.1 refuse a plain `<img>` from `rbl://` to it without
    // ever asking the app. jsdom's page is `http:` as well.
    const agent = vi.spyOn(window.navigator, "userAgent", "get");
    try {
      agent.mockReturnValue("Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/60.5 Safari/605.1.15");
      mount("file:/run/media/dj/STICK/Contents/A B/x y.aiff");
      expect(image()?.getAttribute("src")).toBe("rbl://localhost/artwork/file%3A%2Frun%2Fmedia%2Fdj%2FSTICK%2FContents%2FA%20B%2Fx%20y.aiff");
      expect(image()?.getAttribute("crossorigin")).toBe("anonymous");

      agent.mockReturnValue("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)");
      mount("8");
      expect(image()?.hasAttribute("crossorigin")).toBe(false);
    } finally {
      agent.mockRestore();
    }
  });

  it("shows it once it loads", () => {
    mount();
    act(() => {
      image()?.dispatchEvent(new Event("load"));
    });
    expect(image()?.getAttribute("data-state")).toBe("shown");
  });

  it("asks again after a failure, on a source the browser has not failed on", () => {
    vi.useFakeTimers();
    mount();
    const first = image()?.getAttribute("src");
    expect(first).toBe("rbl://localhost/artwork/7");

    act(() => {
      image()?.dispatchEvent(new Event("error"));
    });
    // Still pending, not given up: the library may simply not be open yet.
    expect(image()?.getAttribute("data-state")).toBe("pending");

    act(() => {
      vi.advanceTimersByTime(250);
    });
    expect(image()?.getAttribute("src")).toBe("rbl://localhost/artwork/7?retry=1");
  });

  it("gives up once the attempts run out, so the ground behind it can show", () => {
    vi.useFakeTimers();
    mount();
    for (let at = 0; at < ARTWORK_RETRIES; at++) {
      act(() => {
        image()?.dispatchEvent(new Event("error"));
      });
      act(() => {
        vi.advanceTimersByTime(250 * 2 ** at);
      });
    }
    expect(image()?.getAttribute("data-state")).toBe("pending");

    act(() => {
      image()?.dispatchEvent(new Event("error"));
    });
    expect(image()?.getAttribute("data-state")).toBe("failed");
  });

  it("still shows a sleeve that arrives after an earlier attempt failed", () => {
    vi.useFakeTimers();
    mount();
    act(() => {
      image()?.dispatchEvent(new Event("error"));
    });
    act(() => {
      vi.advanceTimersByTime(250);
    });
    act(() => {
      image()?.dispatchEvent(new Event("load"));
    });
    expect(image()?.getAttribute("data-state")).toBe("shown");
  });

  it("shows a sleeve the browser had already cached", () => {
    // A cached image is complete before React attaches its handlers and never
    // fires `load` again. Missing that left the square empty for good — worse
    // than the glyph this replaced.
    const complete = Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, "complete");
    const natural = Object.getOwnPropertyDescriptor(HTMLImageElement.prototype, "naturalWidth");
    Object.defineProperty(HTMLImageElement.prototype, "complete", { get: () => true, configurable: true });
    Object.defineProperty(HTMLImageElement.prototype, "naturalWidth", { get: () => 64, configurable: true });
    try {
      mount();
      expect(image()?.getAttribute("data-state")).toBe("shown");
    } finally {
      if (complete) Object.defineProperty(HTMLImageElement.prototype, "complete", complete);
      if (natural) Object.defineProperty(HTMLImageElement.prototype, "naturalWidth", natural);
    }
  });

  it("forgets the last track when a row is recycled", () => {
    mount("7");
    act(() => {
      image()?.dispatchEvent(new Event("load"));
    });
    expect(image()?.getAttribute("data-state")).toBe("shown");

    // Virtualised rows reuse their elements, and the next track's sleeve must
    // not inherit the last one's.
    mount("8");
    expect(image()?.getAttribute("data-state")).toBe("pending");
    expect(image()?.getAttribute("src")).toBe("rbl://localhost/artwork/8");
  });
});
