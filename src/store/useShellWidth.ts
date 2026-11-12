/**
 * The view shell's own width in CSS pixels (wbx-60), as its container
 * queries see it: the window's width divided by the interface scale, which
 * is the webview's zoom in the app and CSS `zoom` in a browser tab (where
 * `matchMedia` would not see it). `ViewShell` hands its root to
 * `observeShellWidth`; whatever must decide in script what the stylesheet
 * decides by `@container shell` (the Library's columns, the second list)
 * reads `useShellWidth`. `null` until the shell is drawn.
 */
import { useSyncExternalStore } from "react";

let width: number | null = null;
const listeners = new Set<() => void>();
let observer: ResizeObserver | null = null;

function set(next: number | null): void {
  const rounded = next === null ? null : Math.round(next);
  if (rounded === width) return;
  width = rounded;
  for (const listener of listeners) listener();
}

/** A ref callback for the shell's root: follows its width while it is mounted. */
export function observeShellWidth(element: HTMLElement | null): void {
  observer?.disconnect();
  observer = null;
  if (element === null) {
    set(null);
    return;
  }
  if (typeof ResizeObserver === "undefined") return;
  observer = new ResizeObserver((entries) => {
    const entry = entries[entries.length - 1];
    if (entry) set(entry.contentRect.width);
  });
  observer.observe(element);
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useShellWidth(): number | null {
  return useSyncExternalStore(subscribe, () => width, () => null);
}
