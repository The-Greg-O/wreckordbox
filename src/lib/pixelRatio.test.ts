/**
 * @vitest-environment jsdom
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { onPixelRatioChange, pixelRatio } from "./pixelRatio";

/** A stand-in for `matchMedia` that remembers each query and its listeners. */
function fakeMedia() {
  const made: { query: string; listeners: Set<() => void> }[] = [];
  const matchMedia = vi.fn((query: string) => {
    const entry = { query, listeners: new Set<() => void>() };
    made.push(entry);
    return {
      matches: true,
      media: query,
      addEventListener: (_: string, listener: () => void) => entry.listeners.add(listener),
      removeEventListener: (_: string, listener: () => void) => entry.listeners.delete(listener),
    };
  });
  return { made, matchMedia };
}

const setRatio = (ratio: number) => Object.defineProperty(window, "devicePixelRatio", { value: ratio, configurable: true });

describe("a change of the display's own ratio", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    setRatio(1);
  });

  it("is told to whoever listens, and the next change is watched for at the new ratio", () => {
    const media = fakeMedia();
    vi.stubGlobal("matchMedia", media.matchMedia);
    window.matchMedia = media.matchMedia as unknown as typeof window.matchMedia;
    setRatio(2);
    const heard = vi.fn();
    const stop = onPixelRatioChange(heard);
    expect(media.made.map((m) => m.query)).toEqual(["(resolution: 2dppx)"]);

    // Moved to a 3x display, same window size.
    setRatio(3);
    for (const listener of [...media.made[0]!.listeners]) listener();
    expect(heard).toHaveBeenCalledTimes(1);
    expect(pixelRatio()).toBe(3);
    expect(media.made.map((m) => m.query)).toEqual(["(resolution: 2dppx)", "(resolution: 3dppx)"]);
    expect(media.made[0]!.listeners.size).toBe(0);

    // And back.
    setRatio(2);
    for (const listener of [...media.made[1]!.listeners]) listener();
    expect(heard).toHaveBeenCalledTimes(2);

    stop();
    expect(media.made.every((m) => m.listeners.size === 0)).toBe(true);
  });
});
