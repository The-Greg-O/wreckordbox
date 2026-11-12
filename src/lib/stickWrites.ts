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
import { parseDeviceNodeId, type DeviceNodeRef } from "./deviceLibrary";
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
const tracksPossessive = (n: number) => `${n} track${n === 1 ? "'s" : "s'"}`;

function megabytes(bytes: number): string {
  return `${Math.max(1, Math.round(Math.abs(bytes) / 1_000_000))} MB`;
}

/** Changed tracks listed by name before the rest are counted. */
const LISTED_CHANGES = 10;

/**
 * The plan in plain lines (wbx-45.4, second review, finding 6): audio
 * copied, audio reused (whose tracks' data is still written), each track
 * whose exported data changes and in which fields, cues and analysis
 * (written, not itemised), then playlists created, then playlists written
 * again with what they gain and lose, then space.
 */
export function describeStickPlan(plan: StickWritePlan): string[] {
  const out: string[] = [];
  if (!plan.audioEstimated) {
    out.push("Audio not estimated: the Stick's library could not be read for it without writing to the Stick");
  }
  if (plan.tracksNew > 0) out.push(`${tracksPossessive(plan.tracksNew)} audio copied to the Stick`);
  if (plan.tracksChanged > 0) out.push(`${tracksPossessive(plan.tracksChanged)} audio copied again (changed at ${plan.tracksChanged === 1 ? "its" : "their"} source)`);
  if (plan.audioReused > 0) out.push(`${tracksPossessive(plan.audioReused)} audio already on the Stick, reused; their data is written again`);
  for (const change of plan.trackChanges.slice(0, LISTED_CHANGES)) {
    out.push(`“${change.title}”: ${change.fields.join(", ")} changed`);
  }
  if (plan.trackChanges.length > LISTED_CHANGES) {
    const rest = plan.trackChanges.length - LISTED_CHANGES;
    out.push(`…and ${rest} more track${rest === 1 ? "" : "s"} whose data changes`);
  }
  if (plan.analysisWritten > 0) {
    out.push(`Cues and analysis of ${tracks(plan.analysisWritten)} written from the Library (changes to them are not listed)`);
  }
  if (plan.tracksRemoved > 0) out.push(`${tracks(plan.tracksRemoved)} removed from the Stick`);
  if (plan.tracksMissing > 0) out.push(`${tracks(plan.tracksMissing)} not written: ${plan.tracksMissing === 1 ? "its file is" : "their files are"} missing`);
  for (const p of plan.playlists) {
    if (p.change !== "created") continue;
    out.push(p.folder || p.added === null ? `${p.folder ? "Folder" : "Playlist"} “${p.name}” created` : `Playlist “${p.name}” created with ${tracks(p.added)}`);
  }
  for (const p of plan.playlists) {
    if (p.change !== "resynchronised") continue;
    const kind = p.folder ? "Folder" : "Playlist";
    if (p.folder) out.push(`${kind} “${p.name}” written again`);
    else if (p.added === null || p.removed === null) {
      out.push(`${kind} “${p.name}” written again with the tracks the Library holds (its entries on the Stick could not be compared)`);
    } else if (p.added === 0 && p.removed === 0) {
      out.push(p.reordered === true
        ? `${kind} “${p.name}” written again: the same tracks in another order`
        : `${kind} “${p.name}” written again, its tracks unchanged`);
    } else {
      out.push(`${kind} “${p.name}” written again: ${tracks(p.added)} added, ${p.removed} removed${p.reordered === true ? ", and the tracks it keeps in another order" : ""}`);
    }
  }
  if (plan.adding !== null) out.push(`${plan.adding} ${plan.adding === 1 ? "entry goes" : "entries go"} into the playlist`);
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

/** How many playlists are under a Stick folder, from the outline the tree shows. */
export function stickPlaylistsUnder(sticks: readonly ShellStick[], ref: DeviceNodeRef): number {
  const library = sticks.find((s) => s.path === ref.path)?.outline?.libraries.find((l) => l.format === ref.format);
  if (!library) return 0;
  const at = library.nodes.findIndex((n) => n.id === ref.id);
  const top = library.nodes[at];
  if (top === undefined) return 0;
  let count = 0;
  for (const n of library.nodes.slice(at + 1)) {
    if (n.depth <= top.depth) break;
    if (!n.folder) count += 1;
  }
  return count;
}
