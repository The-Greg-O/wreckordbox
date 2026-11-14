/**
 * The Stick view's big hold-to-load button and its key, `L` (wbx-72;
 * docs/design/screens/stick). The hold is `holdToLoad.ts`'s machine: a
 * press starts it, only that input letting go ends it, and it is full after
 * one second for a Spare stick or two for a Gig stick; the ring fills while
 * it is held. Released early, or interrupted (the window losing focus or
 * being hidden, the button losing the keyboard), nothing happens. A full
 * hold calls `onHeld` once.
 *
 * `L` is the view's while it is shown (it is the deck's Memory Cue 9 in the
 * Export key map, so the press stops here), except in a text field, or
 * while a dialog or the command palette is open. Only a fresh press counts:
 * a key repeat, as an `L` held down from before the view appeared sends,
 * never starts a hold. Space and Enter on the focused button hold it too
 * (the view owns their activation, so they never reach the deck).
 */
import { useEffect, useRef, useState } from "react";

import { holdMs, IDLE, loadStep, type Holder, type LoadEvent, type LoadMachine } from "@/lib/holdToLoad";
import { isTyping } from "@/lib/shortcuts";
import { PALETTE_SURFACE } from "@/lib/palette";
import styles from "./StickHoldButton.module.css";

/** A dialog or the command palette has the input: `L` is not the view's then. */
export function dialogHasInput(target: EventTarget | null): boolean {
  const modal = `dialog, [role="dialog"], [role="alertdialog"], [${PALETTE_SURFACE}]`;
  if (target instanceof Element && target.closest(modal) !== null) return true;
  if (typeof document === "undefined") return false;
  return document.querySelector(`dialog[open], [role="alertdialog"], [role="dialog"][aria-modal="true"], [${PALETTE_SURFACE}]`) !== null;
}

/** Whether a key event is the hold key, `L` with no modifier. */
function isHoldKey(event: KeyboardEvent): boolean {
  return (event.key === "l" || event.key === "L") && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey;
}

export function StickHoldButton({ name, kind, enabled, why, onHeld }: {
  name: string;
  kind: "gig" | "spare";
  /** False while the load cannot start (nothing chosen, too little space, protected, a load running). */
  enabled: boolean;
  /** Why it cannot start, said under it. */
  why: string | null;
  onHeld: () => void;
}) {
  const [machine, setMachine] = useState<LoadMachine>(IDLE);
  const live = useRef(machine);
  const timer = useRef<number | null>(null);
  const held = useRef(onHeld);
  held.current = onHeld;
  const allowed = useRef(enabled);
  allowed.current = enabled;
  const kindRef = useRef(kind);
  kindRef.current = kind;

  const send = (event: LoadEvent) => {
    const next = loadStep(live.current, event);
    if (next === live.current) return;
    if (next.state !== "holding" && timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    if (next.state === "loading") {
      // The hold is full: the load is the shell's from here; this button is idle again.
      live.current = IDLE;
      setMachine(IDLE);
      held.current();
      return;
    }
    live.current = next;
    setMachine(next);
    if (next.state === "holding" && event.type === "press") {
      timer.current = window.setTimeout(() => {
        timer.current = null;
        send({ type: "tick", at: performance.now() });
      }, next.needs);
    }
  };
  const sender = useRef(send);
  sender.current = send;

  const press = (holder: Holder) => {
    if (!allowed.current) return;
    sender.current({ type: "press", holder, at: performance.now(), kind: kindRef.current });
  };

  // Disabled while holding (the plan changed, a load started elsewhere): the hold ends.
  useEffect(() => { if (!enabled) sender.current({ type: "interrupt" }); }, [enabled]);

  // `L`, heard on the way down ahead of the deck; the window losing focus or hidden interrupts.
  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (!isHoldKey(event) || isTyping(event.target as HTMLElement | null) || dialogHasInput(event.target)) return;
      event.preventDefault();
      event.stopPropagation();
      if (event.repeat || !allowed.current) return;
      sender.current({ type: "press", holder: { key: "l" }, at: performance.now(), kind: kindRef.current });
    };
    const up = (event: KeyboardEvent) => {
      if (event.key !== "l" && event.key !== "L") return;
      if (live.current.state === "holding") {
        event.stopPropagation();
        sender.current({ type: "release", holder: { key: "l" } });
      }
    };
    const lost = () => { sender.current({ type: "interrupt" }); };
    const hidden = () => { if (document.visibilityState !== "visible") lost(); };
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    window.addEventListener("blur", lost);
    document.addEventListener("visibilitychange", hidden);
    return () => {
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
      window.removeEventListener("blur", lost);
      document.removeEventListener("visibilitychange", hidden);
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = null;
    };
  }, []);
  const needs = holdMs(kind);
  const seconds = needs / 1000;
  const holding = machine.state === "holding";
  return (
    <div className={styles.hold}>
      <button
        type="button"
        className={styles.button}
        data-testid="stick-load"
        disabled={!enabled}
        data-holding={holding || undefined}
        aria-describedby={why !== null ? "stick-load-why" : "stick-load-how"}
        style={{ "--hold-ms": `${needs}ms` } as React.CSSProperties}
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          event.currentTarget.setPointerCapture?.(event.pointerId);
          press({ pointer: event.pointerId });
        }}
        onPointerUp={(event) => { send({ type: "release", holder: { pointer: event.pointerId } }); }}
        onPointerCancel={(event) => { send({ type: "release", holder: { pointer: event.pointerId } }); }}
        onKeyDown={(event) => {
          if (event.key !== "Enter" && event.key !== " ") return;
          // The button's own: never the deck's Play/Pause or Load.
          event.preventDefault();
          event.stopPropagation();
          if (event.repeat) return;
          press({ key: event.key });
        }}
        onKeyUp={(event) => {
          if (event.key !== "Enter" && event.key !== " ") return;
          event.preventDefault();
          event.stopPropagation();
          send({ type: "release", holder: { key: event.key } });
        }}
        onClick={(event) => { event.preventDefault(); }}
        onBlur={() => { if (live.current.state === "holding" && "key" in live.current.holder && live.current.holder.key !== "l") send({ type: "interrupt" }); }}
      >
        <svg className={styles.ring} viewBox="0 0 100 100" aria-hidden>
          <circle className={styles.track} cx="50" cy="50" r="46" pathLength={100} />
          <circle className={styles.fill} cx="50" cy="50" r="46" pathLength={100} />
        </svg>
        <span className={styles.label}>
          <span className={styles.big}>Hold to load</span>
          <span className={styles.small}>hold {seconds} s · {kind === "spare" ? "Spare stick" : "Gig stick"}</span>
        </span>
      </button>
      <p id="stick-load-how" className={styles.how}>
        <kbd>Hold L</kbd> or press and hold
        <br />
        {holding ? `Keep holding to load ${name}…` : "release early to cancel"}
      </p>
    </div>
  );
}
