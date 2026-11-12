/**
 * How many device pixels one CSS pixel covers in this window.
 *
 * The whole-interface scale (`uiScale.ts`) is the webview's zoom, and what
 * it does to `devicePixelRatio` depends on the engine. WebKitGTK and
 * Chromium fold it in: at 0.9 on a 1× display `devicePixelRatio` reads 0.9
 * [OBS WebKitGTK, aarch64 Linux, through the test port; Chromium by its
 * documentation]. Safari's page zoom leaves it at the display's, and
 * WKWebView is taken to do the same [ASSUME: not run on macOS]; so does the
 * CSS `zoom` a plain browser tab gets. Where it is left alone, a canvas
 * sized from `devicePixelRatio` is stretched or squeezed onto the pixels it
 * really covers, which blends one-pixel bars, beat grid lines and cue
 * markers into their neighbours. So the zoom is kept here, and everything
 * that draws in device pixels asks `pixelRatio()`.
 *
 * Kept apart from `uiScale.ts` so `canvasSize.ts` and the deck need nothing
 * of the preferences to ask.
 */
import { useSyncExternalStore } from "react";

let zoom = 1;
let zoomInDevicePixelRatio = false;
const listeners = new Set<() => void>();

/**
 * The window's zoom, and whether `devicePixelRatio` already includes it.
 * Anything that is not a positive number is no zoom at all.
 */
export function setPixelZoom(factor: number, inDevicePixelRatio = false): void {
  zoom = Number.isFinite(factor) && factor > 0 ? factor : 1;
  zoomInDevicePixelRatio = inDevicePixelRatio;
  pixelRatioChanged();
}

/** The window's zoom: what the display's own ratio is multiplied by. */
export function pixelZoom(): number {
  return zoom;
}

/** Device pixels per CSS pixel: the display's ratio times the window's zoom. */
export function pixelRatio(): number {
  const reported = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;
  return zoomInDevicePixelRatio ? reported : reported * zoom;
}

/**
 * Tells whoever holds a canvas at a fixed CSS size that `pixelRatio()` may
 * have moved. The webview's zoom lands a moment after it is asked for, so
 * `uiScale.ts` calls this again once it has.
 */
export function pixelRatioChanged(): void {
  for (const listener of listeners) listener();
}

/**
 * Watches the display's own ratio while anything listens: moving the window
 * to a display of another density changes `devicePixelRatio` with no resize
 * and no zoom, and nothing else says so. A `resolution` media query matches
 * only the ratio it was made for, so it is made again after each change.
 */
let displayWatch: (() => void) | null = null;

function watchDisplay(): void {
  if (displayWatch || typeof window === "undefined" || typeof window.matchMedia !== "function") return;
  const arm = () => {
    const query = window.matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    const changed = () => {
      query.removeEventListener("change", changed);
      displayWatch = null;
      if (listeners.size > 0) watchDisplay();
      pixelRatioChanged();
    };
    query.addEventListener("change", changed);
    displayWatch = () => query.removeEventListener("change", changed);
  };
  arm();
}

/** Subscribes to `pixelRatioChanged`; the shape `useSyncExternalStore` takes. */
export function onPixelRatioChange(listener: () => void): () => void {
  listeners.add(listener);
  watchDisplay();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      displayWatch?.();
      displayWatch = null;
    }
  };
}

const serverRatio = () => 1;

/** `pixelRatio()`, kept current: a redraw follows a scale or display change. */
export function usePixelRatio(): number {
  return useSyncExternalStore(onPixelRatioChange, pixelRatio, serverRatio);
}
