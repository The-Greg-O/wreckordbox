/**
 * A running Stick load, shown (wbx-72): full, in place of the Stick view,
 * as a plain progress bar naming the stage and the track being written
 * (the loud display, `LoudLoad.tsx`, is wbx-73's; `StickLoadDisplay.tsx`
 * picks one of the two by the `view.loudStickLoad` setting);
 * or shrunk to the status line, where it stays while the DJ works in any
 * view and a click brings it back to the Stick view. `Esc` shrinks it.
 *
 * Both read the load from `src/store/stickLoads.ts`, so the load and its
 * progress belong to the shell, not to whichever view is showing.
 */
import { useEffect } from "react";

import { runCommand } from "@/lib/commands";
import { progressLine, progressPercent, type LoadProgress } from "@/lib/holdToLoad";
import { isTyping } from "@/lib/shortcuts";
import { showStickLoad, shrinkStickLoad, useStickLoads, type StickLoad } from "@/store/stickLoads";
import { usePreferencesContext } from "@/store/usePreferences";
import { dialogHasInput } from "./StickHoldButton";
import styles from "./StickLoadProgress.module.css";

export function Bar({ progress, label }: { progress: LoadProgress | null; label: string }) {
  const percent = progressPercent(progress);
  return (
    <div
      className={styles.bar}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      aria-valuetext={progressLine(progress)}
      data-unknown={percent === null || undefined}
    >
      <span style={{ width: `${percent ?? 0}%` }} />
    </div>
  );
}

/** `Esc` shrinks a running load to the status line (not while a field or a dialog has the key). */
export function useEscapeShrinks(id: number): void {
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (isTyping(event.target as HTMLElement | null) || dialogHasInput(event.target)) return;
      event.preventDefault();
      shrinkStickLoad(id);
    };
    window.addEventListener("keydown", down);
    return () => { window.removeEventListener("keydown", down); };
  }, [id]);
}

/** Swaps the loud display and the plain bar (the `view.loudStickLoad` setting, also in Preferences › View). */
export function DisplaySwitch({ loud }: { loud: boolean }) {
  const { update } = usePreferencesContext();
  return (
    <button type="button" onClick={() => { update("view", { loudStickLoad: !loud }); }}>
      {loud ? "Show a plain bar" : "Show the loud display"}
    </button>
  );
}

/** The plain display: what the load is doing, how far, and the way to shrink it. */
export function PlainLoadProgress({ load, progress }: { load: StickLoad; progress: LoadProgress | null }) {
  const percent = progressPercent(progress);
  return (
    <section className={styles.panel} data-testid="stick-progress" aria-label={`Loading ${load.name}`}>
      <header className={styles.head}>
        <div>
          <h2 className={styles.title}>Loading {load.name}</h2>
          <p className={styles.line} role="status" aria-live="polite">{progressLine(progress)}</p>
        </div>
        <div className={styles.actions}>
          <DisplaySwitch loud={false} />
          <button type="button" className={styles.shrink} onClick={() => { shrinkStickLoad(load.id); }}>
            <kbd>Esc</kbd> shrink to the status line and keep working
          </button>
        </div>
      </header>
      <p className={styles.percent} aria-hidden>{percent === null ? "…" : `${percent}%`}</p>
      <Bar progress={progress} label={`Loading ${load.name}`} />
      <p className={styles.note}>Do not remove {load.name} while it is loading.</p>
    </section>
  );
}

/** A shrunk load in the Stick view's side panel: how far, and the way back. */
export function ShrunkLoad({ load, progress }: { load: StickLoad; progress: LoadProgress | null }) {
  return (
    <div className={styles.shrunk} data-testid="stick-shrunk">
      <p className={styles.line}>Loading {load.name} · {progressLine(progress)}</p>
      <Bar progress={progress} label={`Loading ${load.name}`} />
      <button type="button" onClick={() => { showStickLoad(load.id); }}>Show the load</button>
    </div>
  );
}

/**
 * The status line's part: every running load, a click away from the Stick
 * view; and every load of this session that failed or did not finish,
 * until the DJ acknowledges its report (wbx-72 review), so a failure that
 * ends while the load is shrunk or another view shows is never missed.
 */
export function StatusStickLoad() {
  const loads = useStickLoads().filter((load) => !load.kept && (load.machine.state === "loading" || load.machine.state === "failed"));
  return (
    <>
      {loads.map((load) => {
        const show = () => {
          showStickLoad(load.id);
          runCommand("viewStick");
        };
        if (load.machine.state === "failed") {
          const what = load.machine.outcome?.outcome === "incomplete" ? "the load did not finish" : "the load failed";
          return (
            <button
              key={load.id}
              type="button"
              className={styles.statusFailed}
              data-testid="status-stick-load"
              data-state="failed"
              title={`${load.name}: ${what}. Show the report.`}
              onClick={show}
            >
              {load.name}: {what} · show the report
            </button>
          );
        }
        const progress = load.machine.state === "loading" ? load.machine.progress : null;
        const counts = progress !== null && progress.total > 0 ? `${Math.min(progress.done, progress.total)}/${progress.total}` : progressLine(progress);
        return (
          <button
            key={load.id}
            type="button"
            className={styles.status}
            data-testid="status-stick-load"
            data-state="loading"
            title={`${progressLine(progress)}. Show the load.`}
            onClick={show}
          >
            <span>Loading {load.name}</span>
            <Bar progress={progress} label={`Loading ${load.name}`} />
            <span>{counts}</span>
          </button>
        );
      })}
    </>
  );
}
