import type { IntakeQueue } from "@/lib/shellTree";
import { EmptyView } from "./EmptyView";

/**
 * The Intake view: a queue of tracks to prepare, and everything needed to
 * prepare one. Two queues share it, chosen in the tree: Intake (tracks with
 * no Prep colour) and Problems (tracks whose Prep colour is Problem). Not
 * built yet; this is where it goes (`docs/design/screens/intake`).
 */
export function IntakeView({ queue = "intake" }: { queue?: IntakeQueue | undefined }) {
  return queue === "problems"
    ? <EmptyView title="Problems" note="Tracks marked Problem will queue here to be fixed. Nothing is here yet." />
    : <EmptyView title="Intake" note="Tracks with no Prep colour will queue here to be prepared. Nothing is here yet." />;
}
