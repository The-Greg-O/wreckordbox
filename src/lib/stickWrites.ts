/**
 * Staged Stick writes from the shell's tree (wbx-45.4 review, findings 5
 * and 7): which Stick may be written to now, how long the deliberate hold
 * is, and the plain lines that say what a write would change.
 *
 * The hold follows the Stick load safeguards (docs/design/requirements.md,
 * "Stick"; docs/design/screens/stick): about one second for a Spare stick,
 * two for a Gig stick; releasing early cancels. A Stick is a Spare stick
 * only when it carries the marker; anything else is treated as a Gig stick.
 */
import type { StickWritePlan, TreeNode } from "@/ipc/types";
import { parseDeviceNodeId } from "./deviceLibrary";
import type { ShellStick } from "./shellTree";

/** How long the confirming hold lasts. */
export function stickHoldMs(spare: boolean): number {
  return spare ? 1000 : 2000;
}

export function stickKind(spare: boolean): "Spare stick" | "Gig stick" {
  return spare ? "Spare stick" : "Gig stick";
}

/**
 * Why nothing may be written to this Stick (nor read through the path that
 * settles an unfinished write) now, or null when it may.
 *
 * `checking` is true from a write's start until the tree has read the Stick
 * afresh after it: whatever the write left, finished or not, is known only
 * then. Settling an unfinished write is not offered here (wbx-45.5 owns it).
 */
export function stickRefusal(stick: ShellStick | undefined, checking: boolean): string | null {
  if (stick === undefined) return "That Stick is no longer connected.";
  const name = stick.name;
  if (stick.outline === null) return `${name} has not been read yet. Try again in a moment.`;
  if (stick.outline.pending) {
    return `A write to ${name} did not finish. Settling it is not offered from the tree yet, so nothing was written.`;
  }
  if (checking) return `${name} is being checked after the last write. Try again in a moment.`;
  return null;
}

const tracks = (n: number) => `${n} track${n === 1 ? "" : "s"}`;

function megabytes(bytes: number): string {
  return `${Math.max(1, Math.round(Math.abs(bytes) / 1_000_000))} MB`;
}

/** The plan in plain lines: tracks, then playlists created, then those written again, then space. */
export function describeStickPlan(plan: StickWritePlan): string[] {
  const out: string[] = [];
  if (plan.tracksNew > 0) out.push(`${tracks(plan.tracksNew)} copied to the Stick`);
  if (plan.tracksChanged > 0) out.push(`${tracks(plan.tracksChanged)} on the Stick rewritten`);
  if (plan.tracksRemoved > 0) out.push(`${tracks(plan.tracksRemoved)} removed from the Stick`);
  if (plan.tracksMissing > 0) out.push(`${tracks(plan.tracksMissing)} not written: ${plan.tracksMissing === 1 ? "its file is" : "their files are"} missing`);
  if (plan.tracksKept > 0) out.push(`${tracks(plan.tracksKept)} already there left as they are`);
  for (const p of plan.playlists) {
    if (p.change === "created") out.push(`${p.folder ? "Folder" : "Playlist"} “${p.name}” created`);
  }
  for (const p of plan.playlists) {
    if (p.change === "resynchronised") out.push(`${p.folder ? "Folder" : "Playlist"} “${p.name}” written again from the Library`);
  }
  if (plan.netAdditionalBytes > 0) out.push(`About ${megabytes(plan.netAdditionalBytes)} more used on the Stick`);
  if (plan.netAdditionalBytes < 0) out.push(`About ${megabytes(plan.netAdditionalBytes)} freed on the Stick`);
  return out;
}

/**
 * The node the Library view may open in the shell: a Stick's playlist only
 * while the Stick may be used (opening it reads the Stick through the path
 * that settles an unfinished write); otherwise nothing.
 */
export function shellOpensNode(node: TreeNode | null, refusal: (path: string) => string | null): TreeNode | null {
  if (node === null) return null;
  const ref = parseDeviceNodeId(node.id);
  if (ref === null) return node;
  return refusal(ref.path) === null ? node : null;
}
