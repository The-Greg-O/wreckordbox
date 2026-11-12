import { useEffect, useRef, useState } from "react";
import { getBackend } from "@/ipc/client";
import type { Backend } from "@/ipc/types";

/** What Library Protection needs of the backend. */
export type ProtectionBackend = Pick<
  Backend,
  "libraryProtection" | "recordLibraryProtection" | "onLibraryProtectionChanged"
>;

/**
 * Keeps Library Protection the same in the window and in the Library
 * service. The window's preferences live in its own storage, which the
 * service cannot read, so the switch is also recorded in a file both can
 * reach: the service reads it before every edit a command asks for.
 *
 * - At start the recorded switch wins, so one turned from the command line
 *   while the window was closed is taken up and not overwritten. With
 *   nothing recorded, the window's own preference is recorded.
 * - From then on a change in Preferences is recorded, and a change from the
 *   command line (`wreckordbox protection off`) becomes the preference.
 */
export function useLibraryProtection(
  protectLibrary: boolean,
  setProtectLibrary: (on: boolean) => void,
  connect: () => Promise<ProtectionBackend> = getBackend,
): void {
  // Nothing is recorded until what was recorded has been read.
  const [started, setStarted] = useState(false);
  // A switch being taken up from outside: not recorded back until the
  // preference has caught up with it.
  const takingUp = useRef<boolean | null>(null);
  const current = useRef(protectLibrary);
  current.current = protectLibrary;
  const set = useRef(setProtectLibrary);
  set.current = setProtectLibrary;

  useEffect(() => {
    let live = true;
    let stop: (() => void) | undefined;
    const takeUp = (on: boolean) => {
      if (on === current.current) return;
      takingUp.current = on;
      set.current(on);
    };
    void (async () => {
      const backend = await connect();
      if (!live) return;
      stop = backend.onLibraryProtectionChanged(takeUp);
      const recorded = await backend.libraryProtection().catch(() => null);
      if (!live) return;
      if (recorded !== null) takeUp(recorded);
      setStarted(true);
    })();
    return () => {
      live = false;
      stop?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the backend is fixed for the window's life
  }, []);

  useEffect(() => {
    if (!started) return;
    if (takingUp.current !== null) {
      if (takingUp.current !== protectLibrary) return;
      takingUp.current = null;
    }
    void connect().then((backend) => backend.recordLibraryProtection(protectLibrary)).catch(() => undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- as above
  }, [started, protectLibrary]);
}
