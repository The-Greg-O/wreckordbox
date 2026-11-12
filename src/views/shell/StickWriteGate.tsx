/**
 * Every write to a Stick from the shell's tree is staged here first
 * (wbx-45.4 review, findings 5 and 7).
 *
 * A write is never started by a drop or a menu click alone. The DJ is shown
 * which Stick it is, Gig stick or Spare stick, and exactly what would
 * change (for an export, the export's own dry run, `stickWritePlan`), and
 * the write starts only after the deliberate hold the Stick load asks for:
 * about one second for a Spare stick, two for a Gig stick, releasing early
 * cancels (docs/design/requirements.md, "Stick"). Cancel writes nothing.
 *
 * A Stick that has not been read, whose last write did not finish, or that
 * has just been written to and not yet read again, is refused for writes
 * and for the reads that would settle an unfinished write (opening a Stick
 * playlist, importing one). After every write the Stick is read again with
 * the read-only outline, and only that answer says whether it may be
 * written to again: a write that failed part-way shows as unfinished and is
 * refused from then on. Settling an unfinished write is not offered (it is
 * wbx-45.5's to add as an explicit action).
 */
import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { getBackend } from "@/ipc/client";
import type { StickOutline, StickWriteRequest } from "@/ipc/types";
import type { ShellStick } from "@/lib/shellTree";
import { describeStickPlan, stickHoldMs, stickKind, stickRefusal } from "@/lib/stickWrites";
import styles from "./StickWriteGate.module.css";

export interface StagedStickWrite {
  /** The Stick's mount path. */
  path: string;
  /** What is being done, in a few words: "Create a playlist on TEST". */
  title: string;
  /** What changes, line by line, known without asking the backend. */
  lines: readonly string[];
  /** An export: its dry run is asked for and its lines added. */
  plan?: StickWriteRequest;
  /** The write itself, once confirmed. */
  run: () => Promise<unknown>;
}

/** What is shown for a write whose confirmation comes part-way through a flow. */
export type StickWriteQuestion = Omit<StagedStickWrite, "path" | "run">;

export interface StickWriteGate {
  /** Why the Stick at `path` may not be written to now, or null. */
  refusal: (path: string) => string | null;
  /** Shows the staged write and runs it once held; false when refused or cancelled. */
  stage: (write: StagedStickWrite) => Promise<boolean>;
  /**
   * A flow that reads first and writes later (a drop on a Stick playlist
   * asks about duplicates before anything is written): `body` is run once
   * the Stick may be used, and asks `approve` right before its first write.
   * The Stick is read again afterwards whenever a write was confirmed.
   */
  guarded: (path: string, body: (approve: (question: StickWriteQuestion) => Promise<boolean>) => Promise<unknown>) => Promise<void>;
  /** The dialog, while a write is being staged. */
  element: ReactNode;
}

interface Asking {
  name: string;
  spare: boolean;
  title: string;
  lines: readonly string[];
  answer: (yes: boolean) => void;
}

export function useStickWriteGate({ sticks, refresh, refuse }: {
  sticks: readonly ShellStick[];
  /** Read the Stick again (the outline follows the edit counter); never through the recovering path in the shell. */
  refresh: (path: string) => Promise<void>;
  refuse: (text: string) => void;
}): StickWriteGate {
  // The outline each Stick had when a write to it started: until a newer
  // one arrives, what the write left is not known.
  const [checking, setChecking] = useState<ReadonlyMap<string, StickOutline | null>>(() => new Map());
  useEffect(() => {
    setChecking((current) => {
      let changed = false;
      const next = new Map(current);
      for (const [path, outline] of current) {
        const stick = sticks.find((s) => s.path === path);
        if (stick === undefined || stick.outline !== outline) {
          next.delete(path);
          changed = true;
        }
      }
      return changed ? next : current;
    });
  }, [sticks]);

  const refusal = useCallback(
    (path: string) => stickRefusal(sticks.find((s) => s.path === path), checking.has(path)),
    [sticks, checking],
  );

  const [asking, setAsking] = useState<Asking | null>(null);
  const live = useRef({ sticks, refusal });
  useEffect(() => { live.current = { sticks, refusal }; });

  /** The dry run, the dialog and the hold; true once the DJ held to the end. */
  const ask = useCallback(async (path: string, question: StickWriteQuestion) => {
    const stick = live.current.sticks.find((s) => s.path === path);
    let spare = stick?.outline?.spare === true;
    let lines = [...question.lines];
    if (question.plan) {
      try {
        const backend = await getBackend();
        const plan = await backend.stickWritePlan(question.plan);
        if (plan.pending) {
          refuse(stickRefusal({ path, name: stick?.name ?? path, outline: { pending: true, libraries: [] } }, false) ?? "");
          void refresh(path);
          return false;
        }
        // A Spare stick only when both readings say so.
        spare = spare && plan.spare;
        lines = [...lines, ...describeStickPlan(plan)];
      } catch (error) {
        refuse(error instanceof Error ? error.message : "What the write would change could not be worked out, so nothing was written.");
        return false;
      }
    }
    const yes = await new Promise<boolean>((answer) => {
      setAsking({ name: stick?.name ?? path, spare, title: question.title, lines, answer });
    });
    setAsking(null);
    if (!yes) return false;
    // Checked again: the Stick may have changed while the DJ was deciding.
    const late = live.current.refusal(path);
    if (late !== null) {
      refuse(late);
      return false;
    }
    setChecking((current) => new Map(current).set(path, stick?.outline ?? null));
    return true;
  }, [refresh, refuse]);

  const guarded = useCallback(async (
    path: string,
    body: (approve: (question: StickWriteQuestion) => Promise<boolean>) => Promise<unknown>,
  ) => {
    const stopped = live.current.refusal(path);
    if (stopped !== null) {
      refuse(stopped);
      return;
    }
    let confirmed = false;
    try {
      await body(async (question) => {
        const yes = await ask(path, question);
        confirmed ||= yes;
        return yes;
      });
    } finally {
      if (confirmed) await refresh(path);
    }
  }, [ask, refresh, refuse]);

  const stage = useCallback(async (write: StagedStickWrite) => {
    let done = false;
    await guarded(write.path, async (approve) => {
      if (!(await approve(write))) return;
      await write.run();
      done = true;
    });
    return done;
  }, [guarded]);

  const element = asking ? <HoldDialog {...asking} /> : null;
  return { refusal, stage, guarded, element };
}

/** The staged write, and the hold that confirms it. */
function HoldDialog({ name, spare, title, lines, answer }: Asking) {
  const hold = stickHoldMs(spare);
  const [holding, setHolding] = useState(false);
  const timer = useRef<number | null>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => { cancel.current?.focus(); }, []);
  useEffect(() => () => { if (timer.current !== null) window.clearTimeout(timer.current); }, []);

  const start = () => {
    if (timer.current !== null) return;
    setHolding(true);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      answer(true);
    }, hold);
  };
  const release = () => {
    // Let go before the hold is full: nothing happens.
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    setHolding(false);
  };
  const keys = (event: KeyboardEvent<HTMLElement>) => {
    // The dialog's keys are its own: the deck and the views never hear them.
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      answer(false);
    }
  };
  const seconds = hold / 1000;

  return (
    <div className={styles.backdrop}>
      <div
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="stick-write-title"
        aria-describedby="stick-write-what"
        className={styles.dialog}
        onKeyDown={keys}
        onKeyUp={(event) => { event.stopPropagation(); }}
      >
        <h2 id="stick-write-title" className={styles.title}>Write to {name}</h2>
        <p className={styles.stick} data-stick-kind={spare ? "spare" : "gig"}>
          {name} · {stickKind(spare)}
        </p>
        <div id="stick-write-what">
          <p className={styles.what}>{title}</p>
          <ul className={styles.lines}>
            {lines.map((line) => <li key={line}>{line}</li>)}
          </ul>
        </div>
        <p className={styles.note}>
          Hold the button for {seconds} second{seconds === 1 ? "" : "s"} to write. Letting go early writes nothing.
        </p>
        <div className={styles.buttons}>
          <button ref={cancel} type="button" onClick={() => { answer(false); }}>Cancel</button>
          <button
            type="button"
            className={styles.hold}
            data-holding={holding || undefined}
            style={{ "--hold-ms": `${hold}ms` } as React.CSSProperties}
            onPointerDown={start}
            onPointerUp={release}
            onPointerLeave={release}
            onPointerCancel={release}
            onKeyDown={(event) => {
              if ((event.key === "Enter" || event.key === " ") && !event.repeat) {
                event.preventDefault();
                start();
              }
            }}
            onKeyUp={(event) => {
              if (event.key === "Enter" || event.key === " ") release();
            }}
            onBlur={release}
          >
            <span className={styles.fill} aria-hidden />
            <span className={styles.holdLabel}>Hold to write ({seconds} s)</span>
          </button>
        </div>
      </div>
    </div>
  );
}
