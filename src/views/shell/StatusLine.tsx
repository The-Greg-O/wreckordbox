import type { StatusBarProps } from "@/views/statusbar/StatusBar";
import { stagePercent } from "@/lib/exportProgress";
import { refusal } from "@/lib/menu";
import { useTranslation } from "@/i18n";
import { useServiceActivity } from "./useServiceActivity";
import styles from "./StatusLine.module.css";

/** Shared by all four views, so a view switch never resets background work. */
export function StatusLine(props: StatusBarProps & { serviceProblem: string | null }) {
  const t = useTranslation();
  const service = useServiceActivity();
  const problem = props.serviceProblem ?? service?.problem;
  const runs = problem ? [] : service?.runs ?? [];
  const runText = runs.length === 1 ? runs[0]!.description
    : runs.length > 1 ? t("{count} Runs in progress", { count: runs.length }) : "";
  const analysis = props.analysisProgress;
  const backup = props.backupProgress;
  const exports = props.exports ?? [];
  const jobs = props.jobs ?? [];
  const error = props.error === problem ? null : props.error;
  return (
    <footer className={styles.line} role="status" aria-label="Library service" aria-live="polite">
      <span data-i18n-ignore className={problem ? styles.problem : styles.connection}
        onContextMenu={props.onOpenLog ? (event) => { event.preventDefault(); props.onOpenLog?.(); } : undefined}>
        {problem || (service?.connected ? t("Service connected") : service ? t("Service not connected") : t("Connecting to service…"))}
      </span>
      {runText && <span data-i18n-ignore className={styles.runs} title={runs.map(run => run.description).join("\n")}>{runText}</span>}
      {props.readOnly && <button type="button" title={refusal(props.protectedLibrary ?? false)}
        onClick={props.onExplainReadOnly} onDoubleClick={props.onDisableReadOnly}>Library read-only</button>}
      {props.onCancelAnalysis && <button type="button" onClick={props.onCancelAnalysis}>Stop</button>}
      {jobs.map(job => <span className={styles.work} key={job.id}>
        {job.stoppable && props.onStopJob && <button type="button" disabled={job.state === "stopping"}
          aria-label={`Stop: ${job.label}`} onClick={() => props.onStopJob?.(job.id)}>
          {job.state === "stopping" ? "Stopping…" : "Stop"}
        </button>}
        <span className={styles.clip} title={job.label}>{job.label}{job.state === "queued" ? " · Queued" : job.total > 0 ? ` · ${job.done}/${job.total}` : ""}</span>
      </span>)}
      {exports.length > 0 && <span className={styles.clip} title={exports.map(job => job.name).join("\n")}>
        {exports.length === 1 ? `Loading Stick · ${exports[0]!.name}${stagePercent(exports[0]!) === undefined ? "" : ` · ${stagePercent(exports[0]!)}%`}` : `${exports.length} Stick loads in progress`}
      </span>}
      {backup ? <span className={styles.clip} title={props.backupActivity}>
        Backup{backup.totalBytes > 0 ? ` · ${Math.min(100, Math.floor(100 * backup.copiedBytes / backup.totalBytes))}%` : ""}
      </span> : props.backupActivity && <span className={styles.clip} title={props.backupActivity}>{props.backupActivity}</span>}
      {analysis ? <span className={styles.clip}>
        Analyzing {analysis.completed}/{analysis.total} tracks{analysis.waiting ? " · Queued" : ""}
      </span> : props.activity && <button type="button" className={styles.message} title={props.activity}
        onClick={props.onShowLog}>{props.activity}</button>}
      {!!props.analysisFailures && <span className={styles.clip} title={props.analysisFailed?.map(f => `${f.title}: ${f.reason}`).join("\n")}>
        {props.analysisFailures} failed
      </span>}
      {error && <span className={styles.error}>
        <button type="button" className={styles.message} title={error} onClick={props.onShowLog}>{error}</button>
        {props.onOpenProtection && <button type="button" onClick={props.onOpenProtection}>Open Preferences</button>}
      </span>}
      <span className={styles.selection} title={props.selection}>{props.selection}</span>
    </footer>
  );
}
