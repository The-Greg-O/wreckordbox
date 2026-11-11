/**
 * Drawing this window at the whole-interface scale.
 *
 * The scale is the webview's own zoom rather than a CSS transform or a font
 * size: it scales every pixel measure in the window at once, text stays
 * crisp, and pointer positions and element boxes stay in the same CSS
 * pixels, so hit-testing on the waveforms needs nothing. The framework's
 * zoom hotkeys do the same thing but keep nothing and have no limits, so
 * they are off and the stops in `preferences.ts` are the only way in.
 *
 * Every window (main, Preferences, Sync Manager, Report bug, Log) runs
 * `followUiScale` from `main.tsx` before it draws: it applies the stored
 * stop, and applies it again whenever the stored preferences change, in this
 * window or another.
 */
import { loadPreferences, PREFERENCES_KEY, uiScale } from "./preferences";
import { pixelRatioChanged, setPixelZoom } from "./pixelRatio";
import { detectPlatform } from "./shortcuts";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * Whether the webview's zoom already shows in `devicePixelRatio`; see
 * `pixelRatio.ts` for which engine does what. A browser tab is zoomed with
 * CSS below, which never shows there.
 */
function zoomShowsInDevicePixelRatio(): boolean {
  return isTauri && !detectPlatform().mac;
}

let applied: number | null = null;

/** Draws this window at `factor` times the measured size. */
export function applyUiScale(factor: number): void {
  if (typeof window === "undefined" || factor === applied) return;
  applied = factor;
  // Before the zoom itself: whatever redraws on the change sizes its canvas
  // for the pixels it is about to cover.
  setPixelZoom(factor, zoomShowsInDevicePixelRatio());
  if (!isTauri) {
    // A browser tab and Playwright have no webview to zoom; CSS `zoom` on
    // the root is the nearest thing, so the mock build shows the scale too.
    document.documentElement.style.setProperty("zoom", factor === 1 ? "" : String(factor));
    pixelRatioChanged();
    return;
  }
  void (async () => {
    try {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      await getCurrentWebview().setZoom(factor);
      // Only now does `devicePixelRatio` read the new value, where the
      // engine folds the zoom into it.
      pixelRatioChanged();
    } catch (failure) {
      console.warn("the window would not change its scale", failure);
    }
  })();
}

/**
 * Applies the stored scale now and on every change to the stored
 * preferences. Registered before React mounts, so it has run by the time a
 * view re-renders for the same change.
 */
export function followUiScale(): void {
  if (typeof window === "undefined") return;
  const apply = () => applyUiScale(uiScale(loadPreferences().view.uiScale));
  apply();
  // This window's own change (`usePreferencesStore.update`) …
  window.addEventListener("rbl-preferences", apply);
  // … and another window's, which arrives as a `storage` event.
  window.addEventListener("storage", (event) => {
    if (event.key === PREFERENCES_KEY || event.key === null) apply();
  });
}
