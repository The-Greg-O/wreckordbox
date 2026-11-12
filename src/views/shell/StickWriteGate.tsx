/**
 * Every write to a Stick, from any part of the window, is staged here
 * (wbx-45.4, both reviews).
 *
 * The backend refuses every Stick write that does not carry an approval
 * token, and only `stickWritePlan` issues one: for one operation, on the
 * Stick as it is now, from the Library as it is now. So every surface that
 * writes to a Stick asks here: the plan is fetched (the export's own dry
 * run where there is one), the DJ is shown which Stick (Gig stick or Spare
 * stick), and what changes, and the write runs with the token only after
 * the deliberate hold the Stick load asks for: about one second for a Spare
 * stick, two for a Gig stick (docs/design/requirements.md, "Stick").
 *
 * The hold: started by one press (a pointer or Enter/Space on the button),
 * finished only by holding that same input; key repeat is ignored; letting
 * go early, the window losing focus or being hidden cancels it, and after
 * an interruption only a fresh press starts it again. One request is shown
 * at a time: a new one cancels the one before (its promise settles as
 * cancelled), and each dialog is keyed to its request, so a timer can
 * never confirm another request's write. The dialog keeps the focus.
 *
 * Before anything that would read the Stick through the path that settles
 * an unfinished write, and before every write, the Stick's outline is
 * checked: a Stick unread, with an unfinished write, or not yet read again
 * after a write is refused. Settling an unfinished write is wbx-45.5's.
 */
import { createContext, useCallback, useContext, useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";

import { getBackend } from "@/ipc/client";
import type { StickOutline, StickWriteOperation, StickWritePlan } from "@/ipc/types";
import type { ShellStick } from "@/lib/shellTree";
import { describeStickPlan, stickHoldMs, stickKind, stickRefusal } from "@/lib/stickWrites";
import styles from "./StickWriteGate.module.css";

/** What a staged write shows and plans. */
export interface StickWriteQuestion {
  /** What is being done, in a few words: "Create a playlist on TEST". */
  title: string;
  /** What changes, line by line, known without asking the backend. */
  lines: readonly string[];
  /** The write, as the backend plans and approves it. */
  plan: StickWriteOperation;
}

export interface StagedStickWrite extends StickWriteQuestion {
  /** The Stick's mount path. */
  path: string;
  /** The write itself, given the approval token. */
  run: (approval: string) => Promise<unknown>;
}

export interface StickWriteGate {
  /** Why the Stick at `path` may not be used now, or null. */
  refusal: (path: string) => string | null;
  /** Plans, shows and (once held) runs one write; false when refused or cancelled. */
  stage: (write: StagedStickWrite) => Promise<boolean>;
  /**
   * A flow that reads first and writes later (a drop on a Stick playlist
   * asks about duplicates first): `body` runs once the Stick may be used and
   * asks `approve` right before its first write, which answers the token or
   * null. The Stick is read again afterwards whenever a write was approved.
   */
  guarded: (path: string, body: (approve: (question: StickWriteQuestion) => Promise<string | null>) => Promise<unknown>) => Promise<void>;
  /**
   * One write's approval on its own (the Sync Manager asks one per Stick,
   * then writes them together): the token, or null when refused or not
   * held for. Call `recheck` on the Stick once the write is over.
   */
  approve: (path: string, question: StickWriteQuestion) => Promise<string | null>;
  /** Reads the Stick again after an approved write, so whether it may be used next is known. */
  recheck: (path: string) => Promise<void>;
  /** The dialog, while a write is being staged. */
  element: ReactNode;
}

interface Asking {
  id: number;
  name: string;
  spare: boolean;
  title: string;
  lines: readonly string[];
  answer: (yes: boolean) => void;
}

const GateContext = createContext<StickWriteGate | null>(null);

/** Shares the window's gate with the parts that write to a Stick (the device panel, the Sync Manager). */
export const StickWriteGateProvider = GateContext.Provider;

/** The window's gate; null outside it (a part shown on its own then refuses Stick writes). */
export function useStickWriteGateContext(): StickWriteGate | null {
  return useContext(GateContext);
}

export function useStickWriteGate({ sticks, refresh, refuse }: {
  sticks: readonly ShellStick[];
  /** Read the Stick again (the outline follows the edit counter); never through the recovering path. */
  refresh: (path: string) => Promise<void>;
  refuse: (text: string) => void;
}): StickWriteGate {
  // The outline each Stick had when a write to it was approved: until a
  // newer one arrives, what the write left is not known.
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
  // The newest request; one before it is cancelled.
  const latest = useRef(0);
  const shown = useRef<Asking | null>(null);

  const ask = useCallback(async (path: string, question: StickWriteQuestion): Promise<string | null> => {
    const id = latest.current + 1;
    latest.current = id;
    // One at a time: the request before, shown or still being planned, is cancelled.
    shown.current?.answer(false);
    shown.current = null;
    setAsking(null);
    const stick = live.current.sticks.find((s) => s.path === path);
    let plan: StickWritePlan;
    try {
      const backend = await getBackend();
      plan = await backend.stickWritePlan({ destination: path, operation: question.plan });
    } catch (error) {
      if (latest.current === id) refuse(error instanceof Error ? error.message : "What the write would change could not be worked out, so nothing was written.");
      return null;
    }
    if (latest.current !== id) return null;
    if (plan.pending || plan.approval === null) {
      refuse(stickRefusal({ path, name: stick?.name ?? path, outline: { pending: true, libraries: [] } }, false) ?? "");
      void refresh(path);
      return null;
    }
    const approval = plan.approval;
    // A Spare stick only when both readings say so; anything else is held for as a Gig stick.
    const spare = stick?.outline?.spare === true && plan.spare;
    const lines = [...question.lines, ...describeStickPlan(plan)];
    const yes = await new Promise<boolean>((answer) => {
      const entry: Asking = { id, name: stick?.name ?? path, spare, title: question.title, lines, answer };
      shown.current = entry;
      setAsking(entry);
    });
    if ((shown.current as Asking | null)?.id === id) {
      shown.current = null;
      setAsking(null);
    }
    if (!yes) return null;
    setChecking((current) => new Map(current).set(path, stick?.outline ?? null));
    return approval;
  }, [refresh, refuse]);

  const guarded = useCallback(async (
    path: string,
    body: (approve: (question: StickWriteQuestion) => Promise<string | null>) => Promise<unknown>,
  ) => {
    const stopped = live.current.refusal(path);
    if (stopped !== null) {
      refuse(stopped);
      return;
    }
    let approved = false;
    try {
      await body(async (question) => {
        const token = await ask(path, question);
        approved ||= token !== null;
        return token;
      });
    } finally {
      if (approved) await refresh(path);
    }
  }, [ask, refresh, refuse]);

  const stage = useCallback(async (write: StagedStickWrite) => {
    let done = false;
    await guarded(write.path, async (approve) => {
      const token = await approve(write);
      if (token === null) return;
      await write.run(token);
      done = true;
    });
    return done;
  }, [guarded]);

  const approve = useCallback(async (path: string, question: StickWriteQuestion) => {
    const stopped = live.current.refusal(path);
    if (stopped !== null) {
      refuse(stopped);
      return null;
    }
    return ask(path, question);
  }, [ask, refuse]);

  const element = asking ? <HoldDialog key={asking.id} {...asking} /> : null;
  return { refusal, stage, guarded, approve, recheck: refresh, element };
}

/** The input that started the hold: only it can finish it. */
type Holder = { pointer: number } | { key: string };

/** The staged write, and the hold that confirms it. */
function HoldDialog({ name, spare, title, lines, answer }: Asking) {
  const hold = stickHoldMs(spare);
  const [holding, setHolding] = useState(false);
  const [interrupted, setInterrupted] = useState(false);
  const timer = useRef<number | null>(null);
  const holder = useRef<Holder | null>(null);
  const settled = useRef(false);
  const dialog = useRef<HTMLDivElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);

  const settle = useCallback((yes: boolean) => {
    if (settled.current) return;
    settled.current = true;
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    answer(yes);
  }, [answer]);

  const stop = useCallback((wasInterrupted: boolean) => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    holder.current = null;
    setHolding(false);
    if (wasInterrupted) setInterrupted(true);
  }, []);

  useEffect(() => { cancelButton.current?.focus(); }, []);
  // Settled as cancelled if the dialog goes without an answer (replaced, or the window closing).
  useEffect(() => () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    if (!settled.current) {
      settled.current = true;
      answer(false);
    }
  }, [answer]);
  // The window losing the focus, or being hidden, ends a hold: its release may never arrive here.
  useEffect(() => {
    const lost = () => { if (holder.current !== null) stop(true); };
    const hidden = () => { if (document.visibilityState !== "visible") lost(); };
    window.addEventListener("blur", lost);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      window.removeEventListener("blur", lost);
      document.removeEventListener("visibilitychange", hidden);
    };
  }, [stop]);

  const start = (who: Holder) => {
    if (holder.current !== null || settled.current) return;
    holder.current = who;
    setInterrupted(false);
    setHolding(true);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      if (holder.current !== null) settle(true);
    }, hold);
  };
  const releasedBy = (who: Holder) => {
    const current = holder.current;
    if (current === null) return;
    const same = "pointer" in who ? "pointer" in current && current.pointer === who.pointer : "key" in current && current.key === who.key;
    // Let go before the hold is full: nothing happens.
    if (same) stop(false);
  };

  const keys = (event: KeyboardEvent<HTMLElement>) => {
    // The dialog's keys are its own: the deck and the views never hear them.
    event.stopPropagation();
    if (event.key === "Escape") {
      event.preventDefault();
      settle(false);
      return;
    }
    if (event.key === "Tab") {
      // The focus stays in the dialog.
      const stops = [...(dialog.current?.querySelectorAll<HTMLElement>("button:not(:disabled)") ?? [])];
      if (stops.length === 0) return;
      const at = stops.indexOf(document.activeElement as HTMLElement);
      const next = stops[(at + (event.shiftKey ? -1 : 1) + stops.length) % stops.length];
      event.preventDefault();
      next?.focus();
    }
  };
  const seconds = hold / 1000;

  return (
    <div className={styles.backdrop}>
      <div
        ref={dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="stick-write-title"
        aria-describedby="stick-write-what"
        className={styles.dialog}
        onKeyDown={keys}
        onKeyUp={(event) => { event.stopPropagation(); }}
        onBlur={(event) => {
          // The focus may not leave the dialog while it is open.
          const to = event.relatedTarget as Node | null;
          if (to !== null && !dialog.current?.contains(to)) cancelButton.current?.focus();
        }}
      >
        <h2 id="stick-write-title" className={styles.title}>Write to {name}</h2>
        <p className={styles.stick} data-stick-kind={spare ? "spare" : "gig"}>
          {name} · {stickKind(spare)}
        </p>
        <div id="stick-write-what">
          <p className={styles.what}>{title}</p>
          <ul className={styles.lines}>
            {lines.map((line, at) => <li key={`${at}:${line}`}>{line}</li>)}
          </ul>
        </div>
        <p className={styles.note} role="status">
          {interrupted
            ? "The hold was interrupted. Press and hold again to write."
            : `Hold the button for ${seconds} second${seconds === 1 ? "" : "s"} to write. Letting go early writes nothing.`}
        </p>
        <div className={styles.buttons}>
          <button ref={cancelButton} type="button" onClick={() => { settle(false); }}>Cancel</button>
          <button
            type="button"
            className={styles.hold}
            data-holding={holding || undefined}
            style={{ "--hold-ms": `${hold}ms` } as React.CSSProperties}
            onPointerDown={(event) => {
              if (event.button !== 0) return;
              start({ pointer: event.pointerId });
            }}
            onPointerUp={(event) => { releasedBy({ pointer: event.pointerId }); }}
            onPointerCancel={(event) => { releasedBy({ pointer: event.pointerId }); }}
            onPointerLeave={(event) => { releasedBy({ pointer: event.pointerId }); }}
            onKeyDown={(event) => {
              if (event.key !== "Enter" && event.key !== " ") return;
              // A key held down repeats; only a fresh press starts a hold.
              event.preventDefault();
              if (event.repeat) return;
              start({ key: event.key });
            }}
            onKeyUp={(event) => {
              if (event.key === "Enter" || event.key === " ") releasedBy({ key: event.key });
            }}
            onBlur={() => { if (holder.current !== null) stop(true); }}
          >
            <span className={styles.fill} aria-hidden />
            <span className={styles.holdLabel}>Hold to write ({seconds} s)</span>
          </button>
        </div>
      </div>
    </div>
  );
}
