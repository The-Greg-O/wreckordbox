/**
 * The large player's Stems control (wbx-67; docs/design/screens/intake):
 * with no Stems it is a single "Split" action; while a split runs it shows
 * its progress (also in the status line, with the Run); current Stems say
 * so and are split again only after the DJ confirms; stale Stems are
 * labelled; a failure shows its reason with Retry.
 *
 * The split belongs to the shell's queue (`src/store/stemSplits.ts`), not
 * to this control, so it carries on whatever view or track is shown, and a
 * track is never split twice at once. Starting one waits for nothing and
 * holds up nothing: the Intake view's Mark Clean and next moves on.
 *
 * At half width the control is hidden and "Split Stems" stays in the
 * command palette (the design's "Stems ... move behind the command
 * palette").
 */
import { memo } from "react";

import { getBackend } from "@/ipc/client";
import { splitLine } from "@/lib/stemSplits";
import { stemSplits, useStemSplit, useStemsStatus } from "@/store/stemSplits";
import { useCommands } from "@/store/useCommands";
import styles from "./StemsControl.module.css";

export interface StemsControlProps {
  track: { id: string; title: string } | null;
}

const AGAIN_QUESTION = "This track already has Stems made from its current audio. Splitting again takes a long time and replaces them.";

function madeAt(ms: number | null): string {
  return ms === null ? "" : new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export const StemsControl = memo(function StemsControl({ track }: StemsControlProps) {
  const id = track?.id ?? null;
  const split = useStemSplit(id);
  const { read, retry } = useStemsStatus(id, split?.phase ?? null);

  const value = read.status === "read" ? read.value : null;
  const busy = split?.phase === "queued" || split?.phase === "running";
  const otherClient = !busy && value?.separating === id && id !== null;
  const blocked = value?.unavailable ?? null;
  // What a press of Split would do: nothing to do, a plain split, or one
  // that asks first (current Stems).
  const offer: "none" | "split" | "again" = !track || busy || otherClient || read.status !== "read" || blocked
    ? "none"
    : value?.state === "current" ? "again" : "split";

  const start = async () => {
    if (!track) return;
    if (offer === "again") {
      const yes = await (await getBackend()).confirm(AGAIN_QUESTION, { yes: "Split again", no: "Keep these Stems", title: "Split Stems again?" });
      if (!yes) return;
      stemSplits().request(track.id, track.title, { again: true });
    } else if (offer === "split") {
      stemSplits().request(track.id, track.title);
    }
  };

  useCommands(offer === "none" ? {} : { intakeSplitStems: () => { void start(); return true; } });

  let state: string;
  let line: string;
  if (!track) { state = "empty"; line = "Load a track to split its Stems"; }
  else if (split && split.phase !== "done") { state = split.phase; line = splitLine(split); }
  else if (otherClient) { state = "running"; line = "Being split by another client"; }
  else if (read.status === "reading") { state = "reading"; line = "Reading…"; }
  else if (read.status === "failed") { state = "unread"; line = `Could not read Stems: ${read.message}`; }
  else if (blocked) { state = "unavailable"; line = blocked.message; }
  else if (value?.state === "current") { state = "current"; line = `Stems ready · made ${madeAt(value.madeAt)}`; }
  else if (value?.state === "stale") { state = "stale"; line = `Stale: the audio changed since these Stems were made (${madeAt(value.madeAt)})`; }
  else { state = "none"; line = "No Stems yet"; }

  const percent = split?.phase === "running" && split.fraction !== null ? Math.round(split.fraction * 100) : null;
  return (
    <div className={styles.stems} role="group" aria-label="Stems" data-testid="stems-control" data-state={state}>
      <span className={styles.label}>Stems</span>
      {split?.phase === "running" && (
        <div
          className={styles.bar}
          role="progressbar"
          aria-label="Splitting Stems"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent ?? undefined}
          aria-valuetext={line}
          data-unknown={percent === null || undefined}
        >
          <span style={{ width: `${percent ?? 0}%` }} />
        </div>
      )}
      <span
        className={styles.line}
        data-testid="stems-state"
        role={state === "failed" || state === "unread" ? "alert" : undefined}
        title={line}
      >
        {line}
      </span>
      {state === "unread" && <button type="button" onClick={retry}>Retry</button>}
      {split?.phase === "failed" && <button type="button" onClick={() => { stemSplits().dismiss(split.trackId); }}>Dismiss</button>}
      {(offer !== "none" || !busy) && (
        <button
          type="button"
          className={styles.split}
          disabled={offer === "none"}
          title={offer === "none" ? (blocked?.message ?? line) : offer === "again" ? "Split these Stems again (asks first)" : "Split this track into vocals, drums, bass and other"}
          onClick={() => { void start(); }}
        >
          {split?.phase === "failed" ? "Retry" : offer === "again" ? "Split again…" : "Split"}
        </button>
      )}
    </div>
  );
});
