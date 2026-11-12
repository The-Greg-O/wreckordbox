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

  const read = useCallback(async (path: string) => {
    pending.current.add(path);
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
  useEffect(() => {
    const present = new Set(devices.map((d) => d.path));
    setLoaded((current) => {
      const kept = new Map([...current].filter(([path]) => present.has(path)));
      return kept.size === current.size ? current : kept;
    });
    for (const path of known.current.keys()) {
      if (present.has(path) && !pending.current.has(path)) void read(path);
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
