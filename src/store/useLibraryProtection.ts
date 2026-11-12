import { useEffect, useRef } from "react";
import { getBackend } from "@/ipc/client";
import type { Backend, LibraryProtection } from "@/ipc/types";

/** What Library Protection needs of the backend. */
export type ProtectionBackend = Pick<
  Backend,
  "libraryProtection" | "adoptLibraryProtection" | "setLibraryProtection" | "onLibraryProtectionChanged"
>;

/**
 * Shows Library Protection as the backend holds it. The backend owns the
 * switch: it is recorded where the Library service reads it, and the
 * Library core refuses every change while it is on, whoever asks. The
 * window's preference is only what the window shows.
 *
 * - At start the window offers its stored preference, which the backend
 *   takes up only the first time, while it has never recorded the switch,
 *   and shows what the backend answers.
 * - It listens for changes before it asks, so none made in between is lost.
 * - Every change, from Preferences (which turns it through the backend and
 *   shows it once recorded) or the command line, arrives as a notification
 *   with a revision; one older than what is shown is ignored.
 * - Nothing here ever writes the switch. A preference changed without the
 *   backend (another window's stale copy, say) is put back from what the
 *   backend says.
 * - When the backend cannot be asked, protection is shown on.
 */
export function useLibraryProtection(
  protectLibrary: boolean,
  setProtectLibrary: (on: boolean) => void,
  connect: () => Promise<ProtectionBackend> = getBackend,
): void {
  // The newest state applied, so an older one arriving late is ignored.
  const shown = useRef<LibraryProtection | null>(null);
  const backendRef = useRef<ProtectionBackend | null>(null);
  const current = useRef(protectLibrary);
  current.current = protectLibrary;
  const set = useRef(setProtectLibrary);
  set.current = setProtectLibrary;

  const apply = useRef((now: LibraryProtection) => {
    if (shown.current && now.revision < shown.current.revision) return;
    shown.current = now;
    if (now.protectLibrary !== current.current) set.current(now.protectLibrary);
  });

  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    // What the window had stored when it started: offered once.
    const stored = current.current;
    void (async () => {
      const backend = await connect();
      if (!live) return;
      backendRef.current = backend;
      // Listening first, and only then asking: a turn made between the
      // answer and the listener would otherwise never be heard.
      try {
        const unlisten = await backend.onLibraryProtectionChanged((now) => {
          if (live) apply.current(now);
        });
        if (!live) {
          unlisten();
          return;
        }
        stop = unlisten;
      } catch {
        // Cannot hear turns: shown on, as when the backend cannot be asked.
        if (live && !current.current) set.current(true);
        return;
      }
      try {
        const now = await backend.adoptLibraryProtection(stored);
        if (live) apply.current(now);
      } catch {
        // Not known: shown on, as the backend treats it.
        if (live && !shown.current && !current.current) set.current(true);
      }
    })();
    return () => {
      live = false;
      stop?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the backend is fixed for the window's life
  }, []);

  // A preference that is not what the backend last said: ask the backend
  // again and show its answer, whichever way that goes.
  useEffect(() => {
    const backend = backendRef.current;
    const last = shown.current;
    if (!backend || !last || last.protectLibrary === protectLibrary) return;
    let live = true;
    backend.libraryProtection().then((now) => {
      if (live) apply.current(now);
    }, () => {
      if (live && !current.current) set.current(true);
    });
    return () => {
      live = false;
    };
  }, [protectLibrary]);
}
