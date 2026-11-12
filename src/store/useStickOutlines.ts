/**
 * Each mounted Stick's playlist trees, for the shell's tree (ticket #52).
 *
 * Read with `stickOutline`, which never writes to the Stick: an export that
 * stopped part way is reported, not finished. This is automatic (nobody
 * opened the Stick), so it must not do what only an explicit load may do.
 *
 * One read per Stick when it is mounted, and again only when something says
 * its contents changed: the device listing's summary of its export, its
 * volume, or `revision` (bumped after an edit to a Stick's playlists). A
 * listing that comes back unchanged (the window asks again on every focus)
 * reads nothing.
 *
 * Every read carries a token. Only the latest read of a Stick still mounted
 * is kept: an answer for a Stick pulled out while it was being read, or for
 * one since replaced at the same mount point, is dropped.
 */
import { useEffect, useMemo, useRef, useState } from "react";

import { getBackend } from "@/ipc/client";
import type { Device, StickOutline } from "@/ipc/types";
import type { ShellStick } from "@/lib/shellTree";

/** What, in the device listing, says a Stick's playlists may have changed. */
function signature(device: Device): string {
  return JSON.stringify([device.volumeId ?? null, device.export ?? null]);
}

export function useStickOutlines(devices: readonly Device[], revision = 0): ShellStick[] {
  const [outlines, setOutlines] = useState<ReadonlyMap<string, StickOutline>>(() => new Map());
  // Per mount point: what the listing said when it was last read, and the
  // token of its latest read.
  const mounts = useRef(new Map<string, { signature: string; volume: string | null; token: number }>());
  const tokens = useRef(0);
  const readAt = useRef(revision);

  useEffect(() => {
    const present = new Set(devices.map((device) => device.path));
    const forget: string[] = [];
    for (const path of mounts.current.keys()) {
      if (!present.has(path)) {
        mounts.current.delete(path);
        forget.push(path);
      }
    }
    const everything = readAt.current !== revision;
    readAt.current = revision;

    const read: string[] = [];
    for (const device of devices) {
      const known = mounts.current.get(device.path);
      const now = signature(device);
      if (known !== undefined && known.signature === now && !everything) continue;
      tokens.current += 1;
      const volume = device.volumeId ?? null;
      // Another volume at the same mount point is another Stick: nothing of
      // the last one's is shown while it is read.
      if (known === undefined || known.volume !== volume) forget.push(device.path);
      mounts.current.set(device.path, { signature: now, volume, token: tokens.current });
      read.push(device.path);
    }

    if (forget.length > 0) {
      setOutlines((current) => {
        if (!forget.some((path) => current.has(path))) return current;
        const next = new Map(current);
        for (const path of forget) next.delete(path);
        return next;
      });
    }
    for (const path of read) {
      const token = mounts.current.get(path)?.token;
      void (async () => {
        let outline: StickOutline;
        try {
          outline = await (await getBackend()).stickOutline(path);
        } catch {
          // Unreadable, or gone: a Stick with nothing under it.
          outline = { pending: false, libraries: [] };
        }
        if (mounts.current.get(path)?.token !== token) return;
        setOutlines((current) => new Map(current).set(path, outline));
      })();
    }
  }, [devices, revision]);

  return useMemo(
    () => devices.map((device) => ({ path: device.path, name: device.name, outline: outlines.get(device.path) ?? null })),
    [devices, outlines],
  );
}
