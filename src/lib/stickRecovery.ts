/**
 * A Stick whose last write did not finish (wbx-45.5).
 *
 * Every read of such a Stick rejects with the `unfinishedWrite` error kind
 * instead of settling it, and the window shows that state in place of the
 * Stick's playlists. Settling it writes to the Stick, so it is an action the
 * DJ chooses, worded here: named for the Stick, saying what it will do.
 */
import type { AppErrorDto, TreeNode } from "@/ipc/types";

/** The line shown where the Stick's playlists would be. */
export const UNFINISHED_WRITE = "A write to this Stick did not finish";

const NOTE_PREFIX = "devunfinished:";

/** The Devices tree's line under a Stick whose last write did not finish. */
export function unfinishedWriteNote(path: string, depth: number): TreeNode {
  return { id: `${NOTE_PREFIX}${path}`, name: UNFINISHED_WRITE, kind: "note", depth };
}

/** Whether a tree node is that line (it belongs to Devices, not the Explorer). */
export function isUnfinishedWriteNote(node: TreeNode): boolean {
  return node.kind === "note" && node.id.startsWith(NOTE_PREFIX);
}

/** Whether a rejected read was refused because of an unfinished write. */
export function isUnfinishedWrite(error: unknown): boolean {
  return typeof error === "object" && error !== null && "kind" in error &&
    (error as Partial<AppErrorDto>).kind === "unfinishedWrite";
}

export interface UnfinishedWriteWording {
  /** The notice's accessible name. */
  label: string;
  /** What happened, and that looking changed nothing. */
  what: string;
  /** The button. */
  action: string;
}

export function unfinishedWriteWording(stick: string): UnfinishedWriteWording {
  return {
    label: `Unfinished write on ${stick}`,
    what:
      "An export to this Stick stopped before it finished, so its playlists are not shown. Nothing on it has been changed since. " +
      "Settling finishes that export if everything it wrote is there, or otherwise puts the Stick back as it was before it; " +
      "files another program wrote to the Stick since are kept.",
    action: `Finish or roll back the unfinished write on ${stick}`,
  };
}
