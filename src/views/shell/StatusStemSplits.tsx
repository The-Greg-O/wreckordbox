/**
 * The window's Stem splits in the status line (wbx-67), beside the Run the
 * service lists for a separation: the split running with its progress,
 * how many wait behind it, and any failure with its reason until the DJ
 * dismisses it. Shown in every view; the progress is the shell's queue's
 * (`src/store/stemSplits.ts`), already coalesced.
 */
import { splitLine } from "@/lib/stemSplits";
import { stemSplits, useStemSplits } from "@/store/stemSplits";
import styles from "./StatusStemSplits.module.css";

export function StatusStemSplits() {
  const splits = useStemSplits();
  const running = splits.find((split) => split.phase === "running");
  const queued = splits.filter((split) => split.phase === "queued").length;
  const failed = splits.filter((split) => split.phase === "failed");
  if (!running && queued === 0 && failed.length === 0) return null;
  const percent = running?.fraction == null ? null : Math.round(running.fraction * 100);
  return (
    <span className={styles.splits} data-testid="status-stems">
      {running && (
        <span className={styles.item} title={`${running.title}: ${splitLine(running)}`} data-testid="status-stems-running">
          Stems · {running.title}{percent === null ? "" : ` · ${percent}%`}
          <span className={styles.bar} role="progressbar" aria-label={`Splitting Stems of ${running.title}`}
            aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined}>
            <span style={{ width: `${percent ?? 0}%` }} />
          </span>
        </span>
      )}
      {!running && queued > 0 && <span className={styles.item}>Stems · {queued} queued</span>}
      {running && queued > 0 && <span className={styles.item}>+{queued} queued</span>}
      {failed.map((split) => (
        <span key={split.trackId} className={styles.failed} role="alert" data-testid="status-stems-failed">
          <span className={styles.item} title={split.reason ?? ""}>Stems failed for “{split.title}”: {split.reason}</span>
          <button type="button" aria-label={`Dismiss: Stems failed for ${split.title}`}
            onClick={() => { stemSplits().dismiss(split.trackId); }}>Dismiss</button>
        </span>
      ))}
    </span>
  );
}
