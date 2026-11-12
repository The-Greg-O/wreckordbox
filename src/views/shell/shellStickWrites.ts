/**
 * The shell's tree's Stick actions, each staged through the Stick write gate
 * (wbx-45.4 review, findings 5 and 7). The writes are still the old tree's
 * handlers; what changes is that none of them starts on a bare drop or menu
 * click: the gate checks the Stick, shows what would change and asks for
 * the hold first. Import Playlist writes only to the Library, but reads the
 * Stick through the path that settles an unfinished write, so it is refused
 * for a Stick the gate refuses.
 */
import type { TreeNode } from "@/ipc/types";
import { LIBRARY_NAMES, parseDeviceNodeId } from "@/lib/deviceLibrary";
import type { ShellStick } from "@/lib/shellTree";
import type { ShellTreeActionProps } from "./ShellTreeActions";
import type { StickWriteGate } from "./StickWriteGate";

type Maybe = Promise<void> | undefined;

/** The window's Stick handlers, in the forms that take the shell's confirmation. */
export interface ShellStickHandlers {
  exportDraggedTo: (path: string, approve?: (ids: readonly string[], run: () => Promise<unknown>) => void) => void;
  addDraggedToDevice: (
    node: TreeNode,
    approve?: (change: { adding: number; exportFirst: readonly string[]; files: boolean }) => Promise<boolean>,
  ) => Maybe;
  createOnDevice: (parent: TreeNode, folder: boolean) => Maybe;
  renameOnDevice: (node: TreeNode, name: string) => Maybe;
  deleteOnDevice: (node: TreeNode, confirmed?: boolean) => Maybe;
  exportPlaylist: (node: TreeNode, path: string) => Maybe;
  importFromDevice: (node: TreeNode) => void;
  deleteUnlistedMusic: boolean;
  compatibilityFormat?: "wav" | "aiff" | "mp3" | undefined;
  /** The names a new Stick playlist and folder are given, in the interface's language. */
  untitled: { playlist: string; folder: string };
}

const tracks = (n: number) => `${n} track${n === 1 ? "" : "s"}`;
const done = (promise: Maybe) => promise ?? Promise.resolve();

export function stagedStickActions(
  actions: ShellTreeActionProps,
  gate: StickWriteGate,
  sticks: readonly ShellStick[],
  handlers: ShellStickHandlers,
): ShellTreeActionProps {
  const stickName = (path: string) => sticks.find((s) => s.path === path)?.name ?? path;
  const where = (node: TreeNode) => {
    const ref = parseDeviceNodeId(node.id);
    return ref ? { path: ref.path, library: LIBRARY_NAMES[ref.format] } : null;
  };
  /** Playlists under a Stick folder, from the outline the tree shows. */
  const under = (node: TreeNode) => {
    const ref = parseDeviceNodeId(node.id);
    const library = sticks.find((s) => s.path === ref?.path)?.outline?.libraries.find((l) => l.format === ref?.format);
    if (!ref || !library) return 0;
    const at = library.nodes.findIndex((n) => n.id === ref.id);
    const top = library.nodes[at];
    if (top === undefined) return 0;
    let count = 0;
    for (const n of library.nodes.slice(at + 1)) {
      if (n.depth <= top.depth) break;
      if (!n.folder) count += 1;
    }
    return count;
  };

  return {
    ...actions,
    onExport: (node, path) => {
      void gate.stage({
        path,
        title: `Export the ${node.kind === "folder" ? "folder" : "playlist"} “${node.name}” to ${stickName(path)}`,
        lines: [],
        plan: {
          kind: "playlist", playlist: node.id, destination: path,
          deleteUnlistedMusic: handlers.deleteUnlistedMusic,
          ...(handlers.compatibilityFormat ? { compatibilityFormat: handlers.compatibilityFormat } : {}),
        },
        run: () => done(handlers.exportPlaylist(node, path)),
      });
    },
    onDropTracksOnDevice: (path) => {
      handlers.exportDraggedTo(path, (ids, run) => {
        void gate.stage({
          path,
          title: `Export ${tracks(ids.length)} to ${stickName(path)}`,
          lines: [],
          plan: {
            kind: "tracks", tracks: [...ids], destination: path,
            ...(handlers.compatibilityFormat ? { compatibilityFormat: handlers.compatibilityFormat } : {}),
          },
          run,
        });
      });
    },
    onDropDeviceTracks: (node) => {
      const at = where(node);
      if (!at) return;
      void gate.guarded(at.path, (approve) => done(handlers.addDraggedToDevice(node, (change) => approve({
        title: `Add ${tracks(change.adding)} to “${node.name}” on ${stickName(at.path)}`,
        lines: [
          `${at.library}: ${tracks(change.adding)} added at the end of “${node.name}”`,
          ...(change.files ? [`The ${change.adding === 1 ? "file is" : "files are"} copied to the Stick first`] : []),
          ...(change.exportFirst.length > 0 ? [`${tracks(change.exportFirst.length)} not on the Stick yet, exported first:`] : []),
        ],
        ...(change.exportFirst.length > 0
          ? { plan: {
            kind: "tracks" as const, tracks: [...change.exportFirst], destination: at.path,
            ...(handlers.compatibilityFormat ? { compatibilityFormat: handlers.compatibilityFormat } : {}),
          } }
          : {}),
      }))));
    },
    onDeviceCreate: (parent, folder) => {
      const at = where(parent);
      if (!at) return;
      const name = folder ? handlers.untitled.folder : handlers.untitled.playlist;
      void gate.stage({
        path: at.path,
        title: `Create a ${folder ? "folder" : "playlist"} on ${stickName(at.path)}`,
        lines: [`${folder ? "Folder" : "Playlist"} “${name}” created in ${at.library}${parent.kind === "deviceFolder" ? `, inside “${parent.name}”` : ""}`],
        run: () => done(handlers.createOnDevice(parent, folder)),
      });
    },
    onDeviceRename: (node, name) => {
      const at = where(node);
      if (!at) return;
      void gate.stage({
        path: at.path,
        title: `Rename on ${stickName(at.path)}`,
        lines: [`${at.library}: “${node.name}” renamed “${name}”`],
        run: () => done(handlers.renameOnDevice(node, name)),
      });
    },
    onDeviceDelete: (node) => {
      const at = where(node);
      if (!at) return;
      const what = node.kind === "deviceFolder"
        ? `Folder “${node.name}” and the ${under(node)} playlist${under(node) === 1 ? "" : "s"} in it deleted from ${at.library}`
        : `Playlist “${node.name}” (${tracks(node.childCount ?? 0)}) deleted from ${at.library}`;
      void gate.stage({
        path: at.path,
        title: `Delete from ${stickName(at.path)}`,
        lines: [what, "The tracks stay on the Stick. This cannot be undone."],
        run: () => done(handlers.deleteOnDevice(node, true)),
      });
    },
    onDeviceImport: (node) => {
      const at = where(node);
      if (!at) return;
      // No Stick write and no hold, but refused as a write would be.
      void gate.guarded(at.path, () => {
        handlers.importFromDevice(node);
        return Promise.resolve();
      });
    },
  };
}
