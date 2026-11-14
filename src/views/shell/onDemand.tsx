/**
 * A view whose code is loaded on demand (wbx-45.13): its own chunk, fetched
 * when the window is idle after its first paint (`preload`) or, at the
 * latest, when the view is first drawn.
 *
 * Not `React.lazy`: React 19 holds a Suspense boundary's content back until
 * 300 ms after its fallback showed (see `main.tsx`), which would be a blank
 * the DJ sees on the first switch to a view. Here a view whose code has
 * arrived draws at once, in the same render as the switch.
 *
 * While the code is on the way the working area says so, and the keys
 * pressed meanwhile are kept and handed to the view once it is drawn, so its
 * shortcuts work on the first press. A load that failed is shown as a
 * failure with Retry, never as an empty view.
 */
import { useEffect, useRef, useSyncExternalStore, type ComponentType, type ReactElement } from "react";

import { useTranslation } from "@/i18n";
import { isTyping } from "@/lib/shortcuts";
import { EmptyView } from "./EmptyView";
import styles from "./ViewShell.module.css";

type State<P> =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; view: ComponentType<P> }
  | { status: "failed"; message: string };

export interface OnDemandView<P> {
  /** Draws the view, or what stands in for it until its code arrives. */
  View: (props: P) => ReactElement;
  /** Starts loading the view's code; resolves once it has arrived. */
  preload: () => Promise<void>;
}

export function onDemand<P extends object>(name: string, load: () => Promise<ComponentType<P>>): OnDemandView<P> {
  let state: State<P> = { status: "idle" };
  let loading: Promise<void> | null = null;
  const listeners = new Set<() => void>();
  const set = (next: State<P>) => { state = next; for (const listener of listeners) listener(); };
  const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
  const current = () => state;

  const preload = (): Promise<void> => {
    if (state.status === "ready") return Promise.resolve();
    if (loading) return loading;
    set({ status: "loading" });
    loading = load().then(
      (view) => { loading = null; set({ status: "ready", view }); },
      (error: unknown) => {
        loading = null;
        set({ status: "failed", message: error instanceof Error ? error.message : String(error) });
        throw error;
      },
    );
    return loading;
  };

  function View(props: P): ReactElement {
    const t = useTranslation();
    const now = useSyncExternalStore(subscribe, current, current);
    const kept = useRef<KeyboardEvent[]>([]);

    useEffect(() => { if (state.status === "idle") preload().catch(() => {}); }, []);

    // Keys pressed while the code is on the way: kept from everything after
    // this listener and handed over once the view is drawn. A key something
    // earlier already took (the view keys 1 to 4, the palette) stays taken.
    const waiting = now.status === "idle" || now.status === "loading";
    useEffect(() => {
      if (!waiting) return undefined;
      const keep = (event: KeyboardEvent) => {
        if (event.defaultPrevented || isTyping(event.target as HTMLElement | null)) return;
        event.preventDefault();
        event.stopPropagation();
        kept.current.push(event);
      };
      window.addEventListener("keydown", keep, true);
      return () => window.removeEventListener("keydown", keep, true);
    }, [waiting]);

    // Runs after the view's own effects (a child's run first), so its key
    // listeners are already attached.
    const ready = now.status === "ready";
    useEffect(() => {
      if (!ready) return;
      const keys = kept.current.splice(0);
      for (const key of keys) {
        const at = key.target instanceof Node && key.target.isConnected ? key.target : window;
        at.dispatchEvent(new KeyboardEvent("keydown", {
          key: key.key, code: key.code, location: key.location, repeat: key.repeat,
          ctrlKey: key.ctrlKey, shiftKey: key.shiftKey, altKey: key.altKey, metaKey: key.metaKey,
          bubbles: true, cancelable: true, composed: true,
        }));
      }
    }, [ready]);

    if (now.status === "ready") {
      const Loaded = now.view;
      return <Loaded {...props} />;
    }
    if (now.status === "failed") {
      return (
        <div className={styles.empty} data-testid="on-demand-failed" data-own-activation="" role="alert">
          <h2 className={styles.emptyTitle}>{t(name)}</h2>
          <p className={styles.emptyNote}>{t("This view could not be loaded")}: {now.message}</p>
          <button type="button" onClick={() => { preload().catch(() => {}); }}>{t("Retry")}</button>
        </div>
      );
    }
    return <EmptyView title={name} note="Opening…" />;
  }

  return { View, preload };
}
