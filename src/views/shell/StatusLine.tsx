import type { StatusBarProps } from "@/views/statusbar/StatusBar";
import { stageLabel, stagePercent } from "@/lib/exportProgress";
import { refusal } from "@/lib/menu";
import { useTranslation } from "@/i18n";
import { useServiceActivity } from "./useServiceActivity";
import styles from "./StatusLine.module.css";
import { ThemeFileStatus } from "./ThemeFileStatus";

/** Shared by all four views, so a view switch never resets background work. */
export function StatusLine(props: StatusBarProps) {
  const t = useTranslation();
  const service = useServiceActivity();
  const problem = service?.problem;
  const runs = problem ? [] : service?.runs ?? [];
  const runText = runs.length === 1 ? runs[0]!.description
    : runs.length > 1 ? t("{count} Runs in progress", { count: runs.length }) : "";
  const analysis = props.analysisProgress;
  const backup = props.backupProgress;
  const exports = props.exports ?? [];
  const jobs = props.jobs ?? [];
  const error = props.error === problem ? null : props.error;
  return (
    <div className={styles.line} data-testid="status-line" data-i18n-ignore>
      <span className={styles.version} title={t("Right-click to open the application log")}
        onContextMenu={props.onOpenLog ? (event) => { event.preventDefault(); props.onOpenLog?.(); } : undefined}>
        wreckordbox{props.version ? ` ${props.version}` : ""}
      </span>
      <span className={styles.service} role="status" aria-label={t("Library service")} aria-live="polite">
      {!problem && <span className={styles.connection}>
        {service?.statusUnavailable ? t("Service status unavailable") : service?.connected ? t("Service connected") : service ? t("Service not connected") : t("Connecting to service…")}
      </span>}
      {service?.runsUnavailable && <span className={styles.clip}>{t("Runs unavailable")}</span>}
      {runText && <span className={styles.runs} title={runs.map(run => run.description).join("\n")}>{runText}</span>}
      </span>
      {problem && <span className={styles.problem} role="alert" title={problem}>{t(problem)}</span>}
      {props.readOnly && <button type="button" title={refusal(props.protectedLibrary ?? false)}
        onClick={props.onExplainReadOnly} onDoubleClick={props.onDisableReadOnly}>{t("Library read-only")}</button>}
      {props.onCancelAnalysis && <button type="button" onClick={props.onCancelAnalysis}>{t("Stop")}</button>}
      {jobs.map(job => <span className={styles.work} key={job.id}>
        {job.stoppable && props.onStopJob && <button type="button" disabled={job.state === "stopping"}
          aria-label={`${t("Stop")}: ${job.label}`} onClick={() => props.onStopJob?.(job.id)}>
          {job.state === "stopping" ? t("Stopping…") : t("Stop")}
        </button>}
        <span className={styles.clip} title={job.label}>{job.label}{job.state === "queued" ? ` · ${t("Queued")}` : job.total > 0 ? ` · ${job.done}/${job.total}` : ""}</span>
      </span>)}
      {exports.length > 0 && <span className={styles.clip} title={exports.map(job => `${job.name}: ${stageLabel(job, t)}`).join("\n")}>
        {exports.length === 1 ? `${stageLabel(exports[0]!, t)} · ${exports[0]!.name}${stagePercent(exports[0]!) === undefined ? "" : ` · ${stagePercent(exports[0]!)}%`}` : `${t("{count} Stick loads in progress", { count: exports.length })} · ${[...new Set(exports.map(job => stageLabel(job, t)))].join(", ")}`}
      </span>}
      {backup ? <span className={styles.clip} title={props.backupActivity}>
        {t("Backup")}{backup.totalBytes > 0 ? ` · ${Math.min(100, Math.floor(100 * backup.copiedBytes / backup.totalBytes))}%` : ""}
      </span> : props.backupActivity && <span className={styles.clip} title={props.backupActivity}>{t(props.backupActivity)}</span>}
      {analysis ? <span className={styles.clip}>
        {t("Analyzing {done}/{total} tracks", { done: analysis.completed, total: analysis.total })}{analysis.waiting ? ` · ${t("Queued")}` : ""}
      </span> : props.activity && <button type="button" className={styles.message} title={props.activity}
        onClick={props.onShowLog}>{t(props.activity)}</button>}
      {!!props.analysisFailures && <span className={styles.clip} title={props.analysisFailed?.map(f => `${f.title}: ${f.reason}`).join("\n")}>
        {t("{count} failed", { count: props.analysisFailures })}
      </span>}
      {error && <span className={styles.error} role="alert">
        <button type="button" className={styles.message} title={error} onClick={props.onShowLog}>{t(error)}</button>
        {props.onOpenProtection && <button type="button" onClick={props.onOpenProtection}>{t("Open Preferences")}</button>}
      </span>}
      <ThemeFileStatus />
      <span className={styles.selection} title={props.selection}>{props.selection}</span>
    </div>
  );
}
