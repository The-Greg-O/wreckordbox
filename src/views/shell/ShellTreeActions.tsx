/**
 * The old tree's actions on the shell's tree (wbx-45.4): its context menus,
 * renaming in place, moving a list by dragging, dropping tracks, files,
 * playlists and folders, Eject, and the Playlist menu's commands on the
 * selected list.
 *
 * Nothing here writes. Every action is one of the handlers the window
 * already gives the old tree (`TreeViewProps`), so each Library edit goes
 * through the same edit path and Library Protection as before, and each
 * Stick edit through the same Stick handlers. Which row offers what is
 * decided in `shellTreeActions.ts`; this turns that into events and menus
 * for `ShellTree` to put on its rows.
 */
import { useEffect, useState, type DragEvent, type KeyboardEvent, type ReactNode } from "react";

import { ContextMenu } from "@/components/ContextMenu";
import { EjectIcon } from "@/components/icons";
import { useTranslation } from "@/i18n";
import { TREE_ROOT, type TreeNode } from "@/ipc/types";
import { deviceTreeMenu, SEPARATOR, treeMenu, type DeviceTreeAction, type MenuRow, type TreeAction } from "@/lib/contextMenus";
import { parseDeviceNodeId } from "@/lib/deviceLibrary";
import { devicePath } from "@/lib/devices";
import { publishTreeMenuState } from "@/lib/menuBar";
import type { ShellStick, ShellTreeRow } from "@/lib/shellTree";
import {
  moveDestination, playlistsMenu, shellRowDrop, shellRowMenu, shellRowMovable, shellRowRenamable,
  type ShellDrag, type ShellTreeMenu,
} from "@/lib/shellTreeActions";
import { childrenOf } from "@/lib/tree";
import { useCommands } from "@/store/useCommands";
import { usePreferences } from "@/store/usePreferences";
import { RenameField, TREE_NODE_DRAG, type TreeViewProps } from "@/views/tree/TreeView";
import styles from "./ShellTree.module.css";

/** The old tree's handlers the shell's tree runs. */
export type ShellTreeActionProps = Pick<TreeViewProps,
  | "dragging" | "canDropToDevicePlaylists" | "onDropTracks" | "onDropDeviceTracks" | "draggingToDevice"
  | "onDropTracksOnDevice" | "onDropFiles" | "onExport" | "exportDevices" | "onExportFile"
  | "onCreatePlaylist" | "onCreateFolder" | "onDeleteNode" | "onRenameNode" | "onMoveNode" | "readOnly"
  | "onCreateSmartPlaylist" | "onEditSmartPlaylist" | "onAddArtwork" | "onAddToShortcut" | "onSortItems"
  | "onEjectDevice" | "ejectingDeviceId" | "deviceBusy"
  | "onDeviceCreate" | "onDeviceRename" | "onDeviceDelete" | "onDeviceImport"
>;

type Edge = "above" | "below" | "into";
type EjectAction = "eject";

/** What a row gets from the actions: spread onto the treeitem. */
export interface ShellRowActions {
  draggable: boolean;
  onContextMenu: (event: React.MouseEvent<HTMLElement>) => void;
  onDoubleClick: (event: React.MouseEvent<HTMLElement>) => void;
  onDragStart: (event: DragEvent<HTMLElement>) => void;
  onDragEnd: () => void;
  onDragEnter: (event: DragEvent<HTMLElement>) => void;
  onDragOver: (event: DragEvent<HTMLElement>) => void;
  onDragLeave: (event: DragEvent<HTMLElement>) => void;
  onDrop: (event: DragEvent<HTMLElement>) => void;
  "data-over"?: true;
  "data-move"?: Edge;
}

export interface ShellTreeActionsApi {
  row: (row: ShellTreeRow) => ShellRowActions;
  /** A key on a row the actions answer (F2, the menu key); true when taken. */
  key: (event: KeyboardEvent<HTMLElement>, row: ShellTreeRow) => boolean;
  /** The rename field, while this row is being renamed. */
  rename: (row: ShellTreeRow, name: string) => ReactNode;
  /** Drawn at the end of a row: a Stick's Eject. */
  end: (row: ShellTreeRow) => ReactNode;
  /** Spread onto the Playlists heading. */
  heading: (groupId: string) => { onContextMenu?: (event: React.MouseEvent<HTMLElement>) => void; onDragOver?: (event: DragEvent<HTMLElement>) => void; onDrop?: (event: DragEvent<HTMLElement>) => void; "data-testid"?: string };
  menu: ReactNode;
}

const PLAYLISTS_HEADING = "shell:heading:playlists";

export function useShellTreeActions({ actions, library, sticks, selectedNode, stickRefusal }: {
  actions: ShellTreeActionProps;
  library: readonly TreeNode[];
  sticks: readonly ShellStick[];
  selectedNode: TreeNode | null;
  /** Why a Stick may not be written to now; a Stick it refuses takes no drop and is greyed in Export. */
  stickRefusal?: ((path: string) => string | null) | undefined;
}): ShellTreeActionsApi {
  const t = useTranslation();
  const { advanced: { doubleClickToEdit } } = usePreferences();
  const {
    readOnly = false, deviceBusy = false, dragging = false, draggingToDevice = false, canDropToDevicePlaylists = false,
  } = actions;
  const [menu, setMenu] = useState<{ x: number; y: number; menu: ShellTreeMenu } | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [moving, setMoving] = useState<TreeNode | null>(null);
  const [moveTo, setMoveTo] = useState<{ id: string; edge: Edge } | null>(null);
  const [overId, setOverId] = useState<string | null>(null);

  useEffect(() => {
    if (readOnly) setRenamingId(null);
  }, [readOnly]);
  // A drag that ended anywhere leaves no row marked.
  useEffect(() => {
    if (!dragging) setOverId(null);
  }, [dragging]);

  // The Playlist menu's commands on the selected list, as the old tree
  // registers them; the old tree is not drawn while the shell is.
  const onSelected = (run: ((node: TreeNode) => void) | undefined) => () => {
    if (!run || selectedNode === null) return false;
    if (!["playlist", "smartPlaylist", "folder", "collection"].includes(selectedNode.kind)) return false;
    run(selectedNode);
    return true;
  };
  useCommands({
    createPlaylist: onSelected(actions.onCreatePlaylist),
    createIntelligentPlaylist: onSelected(actions.onCreateSmartPlaylist),
    createFolder: onSelected(actions.onCreateFolder),
    deletePlaylist: onSelected(actions.onDeleteNode),
    exportPlaylistTxt: onSelected(actions.onExportFile && ((node) => actions.onExportFile?.(node, "txt"))),
    exportPlaylistM3u8: onSelected(actions.onExportFile && ((node) => actions.onExportFile?.(node, "m3u8"))),
  }, "main");
  useEffect(() => {
    const parent = selectedNode?.kind === "collection" ? TREE_ROOT : selectedNode?.id;
    publishTreeMenuState("main", {
      kind: selectedNode?.kind ?? null,
      hasChildren: parent !== undefined && childrenOf(library, parent).length > 0,
    });
  }, [selectedNode, library]);
  useEffect(() => () => publishTreeMenuState("main", null), []);

  const beginRename = (row: ShellTreeRow) => {
    if (readOnly || !shellRowRenamable(row)) return;
    const stick = row.kind === "stickPlaylist" || row.kind === "stickFolder";
    if (stick ? deviceBusy || !actions.onDeviceRename : !actions.onRenameNode) return;
    setRenamingId(row.id);
  };

  const openMenu = (row: ShellTreeRow | null, x: number, y: number) => {
    const found = row === null ? playlistsMenu(library) : shellRowMenu(row, sticks);
    if (found) setMenu({ x, y, menu: found });
    return found !== null;
  };

  /** What is in hand, read from the drag and the window's own drag state. */
  const dragOf = (event: DragEvent<HTMLElement>): ShellDrag => {
    const node = event.dataTransfer.types.includes(TREE_NODE_DRAG);
    return {
      tracks: dragging && !node,
      toStick: dragging && draggingToDevice && !node,
      node: node && Boolean(actions.onExport),
      files: !dragging && !node && event.dataTransfer.types.includes("Files") && Boolean(actions.onDropFiles),
    };
  };
  /** What a drop on this row would do now, after the window's own gates. */
  const dropOn = (row: ShellTreeRow, event: DragEvent<HTMLElement>) => {
    const drop = shellRowDrop(row, sticks, dragOf(event));
    if ((drop === "exportTracks" || drop === "exportNode") && deviceBusy) return null;
    if (drop === "exportTracks" || drop === "exportNode" || drop === "stickTracks") {
      const path = drop === "stickTracks" ? (row.node ? parseDeviceNodeId(row.node.id)?.path : undefined) : devicePath(row.id);
      if (path == null || stickRefusal?.(path) != null) return null;
    }
    if (drop === "stickTracks" && (!canDropToDevicePlaylists || !actions.onDropDeviceTracks)) return null;
    if (drop === "exportTracks" && !actions.onDropTracksOnDevice) return null;
    if (drop === "tracks" && !actions.onDropTracks) return null;
    return drop;
  };
  const edgeOf = (row: ShellTreeRow, event: DragEvent<HTMLElement>): Edge => {
    const box = event.currentTarget.getBoundingClientRect();
    const third = box.height / 3;
    if (row.kind === "folder" && event.clientY > box.top + third && event.clientY < box.bottom - third) return "into";
    return event.clientY > box.top + box.height / 2 ? "below" : "above";
  };
  const endMove = () => {
    setMoving(null);
    setMoveTo(null);
  };

  const takeDrag = (row: ShellTreeRow) => (event: DragEvent<HTMLElement>) => {
    // A Stick exports what is dropped on it, checked first: a list picked
    // up here is also a move in hand, and a Stick is no place in the Library.
    const drop = dropOn(row, event);
    if (drop === "exportNode" || drop === "exportTracks") {
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
      setOverId(row.id);
      return;
    }
    if (moving !== null && actions.onMoveNode) {
      if (row.node === undefined || !shellRowMovable(row)) return;
      const edge = edgeOf(row, event);
      if (moveDestination(library, moving, row.node, edge) === null) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      setMoveTo((at) => (at?.id === row.id && at.edge === edge ? at : { id: row.id, edge }));
      return;
    }
    if (drop === null) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setOverId(row.id);
  };

  const dropped = (row: ShellTreeRow) => (event: DragEvent<HTMLElement>) => {
    setOverId(null);
    const drop = dropOn(row, event);
    const path = row.kind === "stick" ? devicePath(row.id) : null;
    if (drop === "exportNode" && path !== null) {
      event.preventDefault();
      const id = event.dataTransfer.getData(TREE_NODE_DRAG);
      endMove();
      const node = library.find((n) => n.id === id);
      if (node && (node.kind === "playlist" || node.kind === "smartPlaylist" || node.kind === "folder")) actions.onExport?.(node, path);
      return;
    }
    if (drop === "exportTracks" && path !== null) {
      event.preventDefault();
      actions.onDropTracksOnDevice?.(path);
      return;
    }
    if (moving !== null && moveTo?.id === row.id && row.node !== undefined) {
      event.preventDefault();
      const at = moveDestination(library, moving, row.node, moveTo.edge);
      const node = moving;
      endMove();
      if (at !== null) actions.onMoveNode?.(node, at.parent, at.index);
      return;
    }
    if (drop === null || row.node === undefined) return;
    event.preventDefault();
    if (drop === "tracks") actions.onDropTracks?.(row.node.id);
    else if (drop === "stickTracks") actions.onDropDeviceTracks?.(row.node);
    else if (drop === "files" && event.dataTransfer.files.length > 0) actions.onDropFiles?.(row.node.id, Array.from(event.dataTransfer.files));
  };

  const row = (row: ShellTreeRow): ShellRowActions => {
    const movable = shellRowMovable(row) && row.node !== undefined && renamingId !== row.id &&
      (Boolean(actions.onMoveNode) || Boolean(actions.onExport));
    return {
      draggable: movable,
      onContextMenu: (event) => {
        if (openMenu(row, event.clientX, event.clientY)) event.preventDefault();
      },
      onDoubleClick: (event) => {
        if (doubleClickToEdit && !event.shiftKey && !event.metaKey && !event.ctrlKey) beginRename(row);
      },
      onDragStart: (event) => {
        if (!movable || row.node === undefined) return;
        // Not a track drag: the list's rows must not read it as one.
        event.stopPropagation();
        event.dataTransfer.effectAllowed = "copyMove";
        event.dataTransfer.setData("text/plain", row.node.id);
        event.dataTransfer.setData(TREE_NODE_DRAG, row.node.id);
        if (actions.onMoveNode) setMoving(row.node);
      },
      onDragEnd: endMove,
      onDragEnter: takeDrag(row),
      onDragOver: takeDrag(row),
      onDragLeave: (event) => {
        if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
        setOverId((id) => (id === row.id ? null : id));
        setMoveTo((at) => (at?.id === row.id ? null : at));
      },
      onDrop: dropped(row),
      ...(overId === row.id ? { "data-over": true as const } : {}),
      ...(moveTo?.id === row.id ? { "data-move": moveTo.edge } : {}),
    };
  };

  const key = (event: KeyboardEvent<HTMLElement>, row: ShellTreeRow) => {
    if (event.key === "F2" && !event.altKey && !event.ctrlKey && !event.metaKey) {
      beginRename(row);
      return true;
    }
    if (event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey)) {
      const box = event.currentTarget.getBoundingClientRect();
      return openMenu(row, box.left + 16, box.bottom);
    }
    return false;
  };

  const rename = (row: ShellTreeRow, name: string) => {
    if (renamingId !== row.id || readOnly || row.node === undefined) return null;
    const node = row.node;
    return (
      // A click in the field is not a click on the row, which would open it.
      <span className={styles.renameWrap} onClick={(event) => { event.stopPropagation(); }}>
        <RenameField
          name={name}
          className={styles.rename}
          onCommit={(next) => {
            if (next !== "" && next !== node.name) {
              if (node.kind === "devicePlaylist" || node.kind === "deviceFolder") actions.onDeviceRename?.(node, next);
              else actions.onRenameNode?.(node, next);
            }
            setRenamingId(null);
          }}
          onCancel={() => { setRenamingId(null); }}
        />
      </span>
    );
  };

  const end = (row: ShellTreeRow) => {
    if (row.kind !== "stick" || !actions.onEjectDevice || row.node === undefined) return null;
    const node = row.node;
    return (
      <button
        type="button"
        tabIndex={-1}
        className={styles.eject}
        aria-label={`${t("Eject")} ${row.name}`}
        title={`${t("Safely eject")} ${row.name}`}
        disabled={deviceBusy}
        data-ejecting={actions.ejectingDeviceId === row.id || undefined}
        onClick={(event) => {
          event.stopPropagation();
          actions.onEjectDevice?.(node);
        }}
      >
        <EjectIcon />
      </button>
    );
  };

  const heading = (groupId: string) => {
    if (groupId !== PLAYLISTS_HEADING) return {};
    const target: ShellTreeRow = { id: "playlists", kind: "folder", name: "Playlists", depth: 0 };
    return {
      "data-testid": "shell-tree-playlists-heading",
      onContextMenu: (event: React.MouseEvent<HTMLElement>) => {
        if (openMenu(null, event.clientX, event.clientY)) event.preventDefault();
      },
      // Files dropped on the heading become playlists at the top, as on the old tree's Playlists root.
      onDragOver: (event: DragEvent<HTMLElement>) => {
        if (dragOf(event).files) event.preventDefault();
      },
      onDrop: (event: DragEvent<HTMLElement>) => {
        if (!dragOf(event).files || event.dataTransfer.files.length === 0) return;
        event.preventDefault();
        actions.onDropFiles?.(target.id, Array.from(event.dataTransfer.files));
      },
    };
  };

  /** Export's Sticks, each greyed while the gate refuses it (unread, unfinished write, being checked). */
  const writableExports = (rows: readonly MenuRow<TreeAction>[]): MenuRow<TreeAction>[] => rows.map((row) => {
    if (row === SEPARATOR || !row.items) return row;
    return {
      ...row,
      items: row.items.map((item) => {
        if (item === SEPARATOR || typeof item.action !== "string" || !item.action.startsWith("exportTo:")) return item;
        const path = item.action.slice("exportTo:".length);
        return actions.onExport && stickRefusal?.(path) == null && !deviceBusy ? item : { ...item, action: null };
      }),
    };
  });

  const renderMenu = (): ReactNode => {
    if (menu === null) return null;
    const close = () => { setMenu(null); };
    const found = menu.menu;
    if (found.kind === "tree") {
      const { node } = found;
      const run = (action: TreeAction) => {
        if (action.startsWith("exportTo:")) {
          actions.onExport?.(node, action.slice("exportTo:".length));
          return;
        }
        switch (action) {
          case "exportM3u8": actions.onExportFile?.(node, "m3u8"); break;
          case "exportTxt": actions.onExportFile?.(node, "txt"); break;
          case "createPlaylist": actions.onCreatePlaylist?.(node); break;
          case "createFolder": actions.onCreateFolder?.(node); break;
          case "createSmartPlaylist": actions.onCreateSmartPlaylist?.(node); break;
          case "editSmartPlaylist": actions.onEditSmartPlaylist?.(node); break;
          case "addArtwork": actions.onAddArtwork?.(node); break;
          case "addToShortcut": actions.onAddToShortcut?.(node); break;
          case "sortItems": actions.onSortItems?.(node, false); break;
          case "rename": {
            const target = node.kind === "collection" ? null : { id: node.id, kind: node.kind === "folder" ? "folder" : node.kind === "smartPlaylist" ? "smartPlaylist" : "playlist", name: node.name, depth: 0, node } as const;
            if (target) beginRename(target);
            break;
          }
          case "delete": actions.onDeleteNode?.(node); break;
          default: break;
        }
      };
      return (
        <ContextMenu<TreeAction>
          x={menu.x}
          y={menu.y}
          rows={writableExports(treeMenu(found.menu, actions.exportDevices, childrenOf(library, found.menu === "collection" ? TREE_ROOT : node.id).length > 0))}
          label={found.menu === "folder" ? "Folder" : found.menu === "collection" ? "Playlists" : "Playlist"}
          context={{ inPlaylist: true, hasFile: true, readOnly }}
          onChoose={run}
          onClose={close}
        />
      );
    }
    const eject = found.eject;
    const ejectRows: MenuRow<EjectAction>[] = eject && actions.onEjectDevice
      ? [{ label: `${t("Eject")} ${eject.name}`, action: deviceBusy ? null : "eject" }]
      : [];
    const deviceRows: MenuRow<DeviceTreeAction>[] = found.kind === "stick" ? [...deviceTreeMenu(found.menu)] : [];
    const rows: MenuRow<DeviceTreeAction | EjectAction>[] = [
      ...ejectRows,
      ...(ejectRows.length > 0 && deviceRows.length > 0 ? [SEPARATOR] : []),
      ...deviceRows,
    ];
    const label = found.kind === "eject" ? "Stick"
      : found.menu === "deviceFolder" ? "Folder" : found.menu === "devicePlaylists" ? "Playlists" : "Playlist";
    return (
      <ContextMenu<DeviceTreeAction | EjectAction>
        x={menu.x}
        y={menu.y}
        rows={rows}
        label={label}
        context={{ inPlaylist: false, hasFile: false, readOnly: readOnly || deviceBusy }}
        onChoose={(action) => {
          if (action === "eject") {
            if (eject) actions.onEjectDevice?.(eject);
            return;
          }
          if (found.kind !== "stick") return;
          const node = found.node;
          switch (action) {
            case "deviceCreatePlaylist": actions.onDeviceCreate?.(node, false); break;
            case "deviceCreateFolder": actions.onDeviceCreate?.(node, true); break;
            case "deviceDelete": actions.onDeviceDelete?.(node); break;
            case "deviceImportPlaylist": actions.onDeviceImport?.(node); break;
          }
        }}
        onClose={close}
      />
    );
  };

  return { row, key, rename, end, heading, menu: renderMenu() };
}
