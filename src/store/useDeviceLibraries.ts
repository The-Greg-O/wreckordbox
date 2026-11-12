/**
 * The Devices tree's sticks and what their own libraries hold.
 *
 * A stick's libraries are read the first time it is opened and kept; they
 * are read again after an edit to them, and whenever the device list
 * changes (an export, a stick plugged in or out), since either can have
 * rewritten them. A stick that is gone is forgotten. Nothing is polled.
 *
 * None of these reads settles a stick's unfinished write: the backend
 * refuses with `unfinishedWrite` and the tree says so under the stick
 * (wbx-45.5). `settle` is the DJ's explicit action that does.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { getBackend } from "@/ipc/client";
import type { Device, DeviceFormat, DeviceLibrary, DevicePlaylistEdit, DevicePlaylistEditResult, TreeNode } from "@/ipc/types";
import { withDeviceLibraries } from "@/lib/deviceLibrary";
import { deviceNodes, devicePath } from "@/lib/devices";
import { isUnfinishedWrite } from "@/lib/stickRecovery";

export interface DeviceLibraries {
  /** The Devices section: each stick, and under an opened one its libraries. */
  nodes: TreeNode[];
  /** A stick was opened: read its libraries, once. */
  expand: (node: TreeNode) => void;
  /**
   * Changes one library's playlists on a stick, then reads the stick again.
   * Rejects with the backend's reason, having changed nothing.
   */
  edit: (path: string, format: DeviceFormat, edit: DevicePlaylistEdit) => Promise<DevicePlaylistEditResult>;
  refresh: (path: string) => Promise<void>;
  /** Bumped after every edit, so a view of a stick's library opens again. */
  revision: number;
  /**
   * Settles the stick's unfinished write (writes to it: only for the action
   * the DJ chose), then reads it again. Rejects with the backend's reason.
   */
  settle: (path: string) => Promise<void>;
}

export function useDeviceLibraries(devices: readonly Device[]): DeviceLibraries {
  const [loaded, setLoaded] = useState<ReadonlyMap<string, readonly DeviceLibrary[]>>(() => new Map());
  const [revision, setRevision] = useState(0);
  const [unfinished, setUnfinished] = useState<ReadonlySet<string>>(() => new Set());
  // Sticks asked for and not yet answered, so a double-click on a twisty
  // does not read the same stick twice.
  const pending = useRef<Set<string>>(new Set());
  const known = useRef(loaded);
  useEffect(() => {
    known.current = loaded;
  }, [loaded]);
  // Every read carries a token, kept per mount point with the volume it was
  // read for. Only the latest read of a Stick still mounted is kept: an
  // answer for a Stick pulled out while it was read, or for one since
  // replaced at the same mount point, is dropped (as `useStickOutlines`).
  const reads = useRef(new Map<string, { token: number; volume: string | null }>());
  const tokens = useRef(0);
  const volumes = useRef(new Map<string, string | null>());

  const read = useCallback(async (path: string) => {
    pending.current.add(path);
    tokens.current += 1;
    const token = tokens.current;
    reads.current.set(path, { token, volume: volumes.current.get(path) ?? null });
    const backend = await getBackend();
    let found: readonly DeviceLibrary[] = [];
    let interrupted = false;
    try {
      found = await backend.deviceLibraries(path);
    } catch (error) {
      // Unreadable, or gone: a stick with nothing under it, as the Explorer
      // shows a folder it may not open. An unfinished write is said.
      interrupted = isUnfinishedWrite(error);
    }
    if (reads.current.get(path)?.token !== token) return;
    pending.current.delete(path);
    setUnfinished((current) => {
      if (current.has(path) === interrupted) return current;
      const next = new Set(current);
      if (interrupted) next.add(path);
      else next.delete(path);
      return next;
    });
    setLoaded((current) => new Map(current).set(path, found));
  }, []);

  const expand = useCallback(
    (node: TreeNode) => {
      const path = devicePath(node.id);
      if (path === null || known.current.has(path) || pending.current.has(path)) return;
      void read(path);
    },
    [read],
  );

  // A stick that left is forgotten; one still here is read again, since the
  // list changes when an export has just rewritten it.
  // Another volume at the same mount point is another stick: it is
  // forgotten too, and its reads in flight with it.
  useEffect(() => {
    const now = new Map(devices.map((d) => [d.path, d.volumeId ?? null] as const));
    const forget = new Set<string>();
    for (const [path, volume] of volumes.current) {
      if (!now.has(path) || now.get(path) !== volume) forget.add(path);
    }
    for (const path of reads.current.keys()) {
      if (!now.has(path)) forget.add(path);
    }
    volumes.current = now;
    for (const path of forget) {
      reads.current.delete(path);
      pending.current.delete(path);
    }
    const kept = (path: string) => now.has(path) && !forget.has(path);
    setLoaded((current) => {
      const next = new Map([...current].filter(([path]) => kept(path)));
      return next.size === current.size ? current : next;
    });
    setUnfinished((current) => {
      const next = new Set([...current].filter(kept));
      return next.size === current.size ? current : next;
    });
    for (const path of known.current.keys()) {
      if (kept(path) && !pending.current.has(path)) void read(path);
    }
  }, [devices, read]);

  const edit = useCallback(
    async (path: string, format: DeviceFormat, change: DevicePlaylistEdit) => {
      const backend = await getBackend();
      const result = await backend.devicePlaylistEdit(path, format, change);
      if (result.changed > 0) {
        await read(path);
        setRevision((r) => r + 1);
      }
      return result;
    },
    [read],
  );

  const refresh = useCallback(async (path: string) => {
    await read(path);
    setRevision((revision) => revision + 1);
  }, [read]);

  const settle = useCallback(async (path: string) => {
    const backend = await getBackend();
    await backend.settleStick(path);
    await refresh(path);
  }, [refresh]);

  const nodes = useMemo(() => withDeviceLibraries(deviceNodes(devices), loaded, unfinished), [devices, loaded, unfinished]);

  return useMemo(() => ({ nodes, expand, edit, refresh, revision, settle }), [nodes, expand, edit, refresh, revision, settle]);
}
