/**
 * Y and N in the Review view (wbx-mvr; decisions.md 2026-11-14 item 5).
 *
 * Y accepts and N rejects the ONE row the keyboard focus is in, through the
 * same decision as that row's ✓ and ✕ (so bound to the row's token in the
 * listing shown). What keeps a key press from deciding the wrong thing:
 * - only the focused row, never the chosen rows or a Run's bulk set;
 * - a key's repeats, and a key already down when the row took focus, do
 *   nothing until it is released and pressed again;
 * - a row that arrived, moved or changed since it took focus is not
 *   decided, and the view says why (the DJ moves the focus off and back to
 *   decide it as it is now);
 * - nothing from a text field, the command palette or a dialog;
 * - a Y or N pressed on a row stops there, so it never reaches the deck
 *   (N is the deck's Next memory cue).
 */
import { useCallback, useEffect, useRef, useState, type FocusEvent, type KeyboardEvent } from "react";

import { isTyping } from "@/lib/shortcuts";
import type { ReviewListing } from "@/lib/reviewQueue";
import { modalOwnsInput } from "@/views/palette/CommandPalette";

const ROW = '[role="row"][data-fact]';

/** What a row was when it took focus, compared again when a key is pressed. */
export interface RowAtFocus {
  fact: number;
  /** The row's token in the listing shown. */
  token: string;
  /** Its position among the rows drawn. */
  index: number;
  /** Its offset from the top of the list, in CSS pixels. */
  top: number;
  /** What it shows. */
  text: string;
  /** `data-marked`: arrived or changed while the DJ was away, or null. */
  marked: string | null;
}

export type SinceFocus = "arrived" | "moved" | "changed";

/** Why the row is not the one that took focus, or `null` when it is. */
export function sinceFocus(atFocus: RowAtFocus, now: RowAtFocus): SinceFocus | null {
  if (now.fact !== atFocus.fact) return "arrived";
  if (now.marked !== atFocus.marked && now.marked === "added") return "arrived";
  if (now.token !== atFocus.token || now.text !== atFocus.text || now.marked !== atFocus.marked) return "changed";
  if (now.index !== atFocus.index || Math.abs(now.top - atFocus.top) > 0.5) return "moved";
  return null;
}

interface KeyLike { key: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean }

/** The decision a key asks for on a row: Y accepts, N rejects; nothing with Ctrl, Cmd or Alt. */
export function rowKeyVerb(event: KeyLike): "accept" | "reject" | null {
  if (event.ctrlKey || event.metaKey || event.altKey) return null;
  const key = event.key.toLowerCase();
  return key === "y" ? "accept" : key === "n" ? "reject" : null;
}

const KEY_NAME = { accept: "Y", reject: "N" } as const;

const SAID: Record<SinceFocus, string> = {
  arrived: "this row arrived since it took focus",
  moved: "this row moved since it took focus",
  changed: "this row changed since it took focus",
};

function snapshot(row: HTMLElement, shown: ReviewListing | null): RowAtFocus {
  const fact = Number(row.dataset.fact);
  const list = row.closest('[data-testid="review-list"]') ?? row.parentElement!;
  const all = [...list.querySelectorAll(ROW)];
  return {
    fact,
    token: shown?.rowTokens[fact] ?? "",
    index: all.indexOf(row),
    top: row.getBoundingClientRect().top - list.getBoundingClientRect().top + list.scrollTop,
    text: row.textContent ?? "",
    marked: row.getAttribute("data-marked"),
  };
}

export interface RowKeysState {
  shown: ReviewListing | null;
  /** The list is held after a refusal: nothing is decided until the DJ shows it. */
  held: boolean;
  busy: boolean;
  protectedLibrary: boolean;
  decide: (verb: "accept" | "reject", facts: number[]) => void;
}

export interface RowKeys {
  onFocusCapture: (event: FocusEvent<HTMLElement>) => void;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
  /** Why the last Y or N did nothing, or `null`. */
  refused: string | null;
  dismiss: () => void;
}

export function useReviewRowKeys(state: RowKeysState): RowKeys {
  const [refused, setRefused] = useState<string | null>(null);
  const live = useRef(state);
  live.current = state;
  // Keys down anywhere in the window, so a key held from before is known.
  const down = useRef(new Set<string>());
  const wasDown = useRef(false);
  const atFocus = useRef<{ row: RowAtFocus; held: Set<string> } | null>(null);

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

  const onFocusCapture = useCallback((event: FocusEvent<HTMLElement>) => {
    const row = (event.target as Element).closest?.<HTMLElement>(ROW) ?? null;
    const fact = row ? Number(row.dataset.fact) : null;
    if (row && atFocus.current?.row.fact === fact) return;
    atFocus.current = row ? { row: snapshot(row, live.current.shown), held: new Set(down.current) } : null;
    setRefused(null);
  }, []);

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLElement>) => {
    const verb = rowKeyVerb(event);
    if (verb === null) return;
    const target = event.target as HTMLElement;
    if (modalOwnsInput(target)) return;
    if (isTyping(target) && !(target instanceof HTMLInputElement && target.type === "checkbox")) return;
    const row = target.closest?.<HTMLElement>(ROW) ?? null;
    if (!row) return;
    // A Y or N on a row is the row's, whatever it does: never the deck's.
    event.preventDefault();
    event.stopPropagation();
    const key = event.key.toLowerCase();
    const name = KEY_NAME[verb];
    const focus = atFocus.current;
    if (!focus || focus.held.has(key)) {
      setRefused(`${name} did nothing: it was already held down when this row took focus. Release it and press it again.`);
      return;
    }
    // A held key's repeats, marked or not (some engines send them unmarked).
    if (event.repeat || wasDown.current) return;
    if (row.hasAttribute("data-ghost")) return;
    const { shown, held, busy, protectedLibrary, decide } = live.current;
    const since = sinceFocus(focus.row, snapshot(row, shown));
    if (since !== null) {
      setRefused(`${name} did nothing: ${SAID[since]}. Move the focus off it and back to decide it as it is now.`);
      return;
    }
    if (held) {
      setRefused(`${name} did nothing: the list is as you saw it before a refusal. Show the list as it is now first.`);
      return;
    }
    if (busy) {
      setRefused(`${name} did nothing: a decision is still being made.`);
      return;
    }
    if (verb === "accept" && protectedLibrary) {
      setRefused(`${name} did nothing: Library Protection is on, so nothing can be accepted.`);
      return;
    }
    setRefused(null);
    decide(verb, [focus.row.fact]);
  }, []);

  return { onFocusCapture, onKeyDown, refused, dismiss: useCallback(() => setRefused(null), []) };
}
