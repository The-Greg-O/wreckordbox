/**
 * Proposed titles and artists in the Intake track panel (wbx-66).
 *
 * A proposal is a row of Review's queue: the service lists it for the
 * on-screen track with the row's own token, and accepting it here is
 * Review's own decision (`review.accept` with that row), so it is the DJ's
 * hand set, refused under Library Protection or a Stick-hosted Library, and
 * refused with a reason if the proposal changed or was decided since it was
 * shown. Each shows beside its field as a button that looks unlike a set
 * value (dashed, marked "proposed"); a click, or Y while it has the focus,
 * accepts it.
 *
 * Y follows Review's rules (wbx-mvr): only the ONE proposal holding the
 * focus; a key's repeats do nothing, and a key already down when the
 * proposal took focus does nothing until it is released; a proposal that
 * changed since it took focus is not accepted (the panel says why); never
 * from a text field or while the palette or a dialog owns the keys; and a
 * Y pressed on a proposal stops there, so it never reaches the deck.
 */
import { useCallback, useEffect, useRef, useState, type FocusEvent, type KeyboardEvent } from "react";

import { useTranslation } from "@/i18n";
import type { IntakeSuggestion } from "@/lib/intakeQueue";
import { isTyping } from "@/lib/shortcuts";
import { modalOwnsInput } from "@/views/palette/CommandPalette";
import styles from "./IntakeTrackPanel.module.css";

const PROPOSAL = "[data-suggestion]";

/** What a proposal was when it took focus, compared again when Y is pressed. */
export interface ProposalAtFocus {
  fact: number;
  token: string;
  value: string;
}

/** Why the focused proposal is not the one that took focus, or `null` when it is. */
export function proposalSince(atFocus: ProposalAtFocus, now: ProposalAtFocus | null): "gone" | "changed" | null {
  if (now === null || now.fact !== atFocus.fact) return "gone";
  if (now.token !== atFocus.token || now.value !== atFocus.value) return "changed";
  return null;
}

/** Whether a key press asks to accept: a plain Y. */
export function acceptsKey(event: { key: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean }): boolean {
  return !event.ctrlKey && !event.metaKey && !event.altKey && event.key.toLowerCase() === "y";
}

export interface SuggestionKeysState {
  /** The proposals shown now. */
  shown: readonly IntakeSuggestion[];
  /** Another operation, or an accept, is under way. */
  busy: boolean;
  protectedLibrary: boolean;
  accept: (suggestion: IntakeSuggestion) => void;
}

/** Y on the focused proposal; the handlers go on the element around the proposals. */
export function useSuggestionKeys(state: SuggestionKeysState) {
  const [refused, setRefused] = useState<string | null>(null);
  const live = useRef(state);
  live.current = state;
  const down = useRef(new Set<string>());
  const wasDown = useRef(false);
  const atFocus = useRef<{ proposal: ProposalAtFocus; held: Set<string> } | null>(null);

  useEffect(() => {
    const onDown = (event: globalThis.KeyboardEvent) => {
      const key = event.key.toLowerCase();
      wasDown.current = down.current.has(key);
      down.current.add(key);
    };
    const onUp = (event: globalThis.KeyboardEvent) => {
      const key = event.key.toLowerCase();
      down.current.delete(key);
      atFocus.current?.held.delete(key);
    };
    const onBlur = () => {
      down.current.clear();
      atFocus.current?.held.clear();
    };
    window.addEventListener("keydown", onDown, true);
    window.addEventListener("keyup", onUp, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onDown, true);
      window.removeEventListener("keyup", onUp, true);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  const find = (fact: number): ProposalAtFocus | null => {
    const s = live.current.shown.find((shown) => shown.fact === fact);
    return s ? { fact: s.fact, token: s.token, value: s.value } : null;
  };

  const onFocusCapture = useCallback((event: FocusEvent<HTMLElement>) => {
    const button = (event.target as Element).closest?.<HTMLElement>(PROPOSAL) ?? null;
    const fact = button ? Number(button.dataset.suggestion) : null;
    if (button && atFocus.current?.proposal.fact === fact) return;
    const now = fact === null ? null : find(fact);
    atFocus.current = now ? { proposal: now, held: new Set(down.current) } : null;
    setRefused(null);
  }, []);

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLElement>) => {
    if (!acceptsKey(event)) return;
    const target = event.target as HTMLElement;
    if (modalOwnsInput(target) || isTyping(target)) return;
    const button = target.closest?.<HTMLElement>(PROPOSAL) ?? null;
    if (!button) return;
    // A Y on a proposal is the proposal's, whatever it does: never the deck's.
    event.preventDefault();
    event.stopPropagation();
    const focus = atFocus.current;
    if (!focus || focus.held.has("y")) {
      setRefused("Y did nothing: it was already held down when this proposal took focus. Release it and press it again.");
      return;
    }
    if (event.repeat || wasDown.current) return;
    const since = proposalSince(focus.proposal, find(Number(button.dataset.suggestion)));
    if (since !== null) {
      setRefused(since === "gone"
        ? "Y did nothing: this proposal is no longer waiting."
        : "Y did nothing: this proposal changed since it took focus. Move the focus off it and back to accept it as it is now.");
      return;
    }
    const { busy, protectedLibrary, accept, shown } = live.current;
    if (busy) {
      setRefused("Y did nothing: an edit is still being made.");
      return;
    }
    if (protectedLibrary) {
      setRefused("Y did nothing: Library Protection is on, so nothing can be accepted.");
      return;
    }
    const chosen = shown.find((s) => s.fact === focus.proposal.fact);
    if (!chosen) return;
    setRefused(null);
    accept(chosen);
  }, []);

  return { onFocusCapture, onKeyDown, refused };
}

/** One field's proposals, beside the field. */
export function FieldSuggestions({ field, suggestions, accepting, busy, onAccept }: {
  /** The field's name as shown ("Title"). */
  field: string;
  suggestions: readonly IntakeSuggestion[];
  /** The Fact being accepted now, if any. */
  accepting: number | null;
  busy: boolean;
  onAccept: (suggestion: IntakeSuggestion) => void;
}) {
  const t = useTranslation();
  if (suggestions.length === 0) return null;
  return (
    <ul className={styles.proposals} aria-label={`${t("Proposed")} ${field}`} data-testid={`intake-proposals-${field.toLowerCase()}`}>
      {suggestions.map((s) => {
        const pending = accepting === s.fact;
        const confidence = s.confidence.toFixed(2);
        return (
          <li key={s.fact}>
            <button
              type="button"
              className={styles.proposal}
              data-suggestion={s.fact}
              data-pending={pending ? "" : undefined}
              // Never `disabled`: a disabled button drops the focus Y depends on.
              aria-disabled={busy || undefined}
              aria-label={`${t("Accept proposed")} ${field} “${s.value}” ${t("from")} ${s.source} (${confidence})`}
              title={`${t("Accept")} (Y)`}
              onClick={(event) => {
                // A double click is one decision.
                if (event.detail > 1 || busy) return;
                onAccept(s);
              }}
            >
              <span className={styles.proposedMark} aria-hidden="true">{t("proposed")}</span>
              <span className={styles.proposedValue}>{s.value}</span>
              <span className={styles.source}>{s.source} · {confidence}</span>
              <kbd className={styles.key} aria-hidden="true">Y</kbd>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
