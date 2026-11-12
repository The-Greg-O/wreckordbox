/**
 * Stick Protection (wbx-45.11), as the window shows it: whether each
 * mounted Stick is protected or unlocked. The backend holds the rule and
 * refuses every write to a protected Stick; this only reads it, and reads
 * it again after an unlock or lock anywhere in the window (or a device
 * list change: an eject or a re-insert ends an unlock).
 */
import { useCallback, useEffect, useState } from "react";

import { getBackend } from "@/ipc/client";
import type { Device, StickProtection } from "@/ipc/types";

const CHANGED = "rbl:stick-protection";

/** Tells every reader that a Stick was unlocked or locked. */
export function announceStickProtection(): void {
  window.dispatchEvent(new Event(CHANGED));
}

/** Unlocks the Stick at `path` (a Gig stick needs its `name` typed) and tells the window. */
export async function unlockStick(path: string, name?: string): Promise<StickProtection> {
  const backend = await getBackend();
  const now = await backend.stickUnlock(path, name);
  announceStickProtection();
  return now;
}

/** Protects the Stick at `path` again and tells the window. */
export async function lockStick(path: string): Promise<StickProtection> {
  const backend = await getBackend();
  const now = await backend.stickLock(path);
  announceStickProtection();
  return now;
}

export function useStickProtection(devices: readonly Device[]): ReadonlyMap<string, StickProtection> {
  const [found, setFound] = useState<ReadonlyMap<string, StickProtection>>(() => new Map());
  const read = useCallback(async () => {
    const backend = await getBackend();
    const read = await Promise.all(devices.map(async (device) => {
      try {
        return [device.path, await backend.stickProtection(device.path)] as const;
      } catch {
        return null;
      }
    }));
    setFound(new Map(read.filter((entry) => entry !== null)));
  }, [devices]);
  useEffect(() => {
    void read();
    const again = () => { void read(); };
    window.addEventListener(CHANGED, again);
    return () => window.removeEventListener(CHANGED, again);
  }, [read]);
  return found;
}
