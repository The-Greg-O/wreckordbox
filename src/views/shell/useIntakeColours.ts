/**
 * The DJ's Prep colours for the Intake view (wbx-63): the Library service's
 * eight colour labels with the DJ's names, and which mean Clean and Problem.
 *
 * Read when the view mounts and again when the Library finishes loading (a
 * read made while it loads fails). A failed read is said, with `retry`; the
 * Prep colour buttons and "Mark Clean and next" are then unavailable rather
 * than guessing which colour is Clean.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import { getBackend } from "@/ipc/client";
import { coalescedRefresh, type Refresher } from "@/lib/coalescedRefresh";
import type { IntakeColours } from "@/lib/intakeEdits";

export interface IntakeColoursState {
  colours: IntakeColours | null;
  /** Why the latest read failed; `null` when it succeeded or is still running. */
  failed: string | null;
  retry: () => void;
}

function messageOf(error: unknown): string {
  if (error instanceof Error) return error.message;
  const message = (error as { message?: unknown } | null)?.message;
  return typeof message === "string" ? message : String(error);
}

export function useIntakeColours(): IntakeColoursState {
  const [state, setState] = useState<{ colours: IntakeColours | null; failed: string | null }>({ colours: null, failed: null });
  const refresher = useRef<Refresher | null>(null);
  useEffect(() => {
    let live = true;
    const stops: (() => void)[] = [];
    const refresh = coalescedRefresh(async () => {
      const backend = await getBackend();
      try {
        const colours = await backend.intakeColours();
        if (live) setState({ colours, failed: null });
      } catch (error) {
        if (live) setState((shown) => ({ ...shown, failed: messageOf(error) }));
      }
    });
    refresher.current = refresh;
    void (async () => {
      const backend = await getBackend();
      if (!live) return;
      stops.push(backend.onLibraryReady(() => refresh.request()));
      refresh.request();
    })();
    return () => {
      live = false;
      refresh.stop();
      for (const stop of stops) stop();
    };
  }, []);
  const retry = useCallback(() => refresher.current?.request(), []);
  return { ...state, retry };
}
