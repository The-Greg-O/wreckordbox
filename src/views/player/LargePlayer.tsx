/**
 * The large player (wbx-62): the Intake view's face of the shell's deck.
 * `docs/design/screens/intake/intake-1280.png` is the render: the compact
 * strip on top (CUE, PLAY, readouts, overview with phrases under it, beat
 * jump), then the phrases above the detail waveform, then the detail
 * waveform with its beat grid and cue markers under the fixed playhead.
 *
 * Presentational, as `CompactPlayer` is: `Player` (with `compact` and
 * `large`) owns the deck and hands this the strip it already draws and the
 * band's moving parts, whose refs its frame loop writes. The eight cue pads,
 * grid nudge and tap tempo under the detail are wbx-64; the per-track tools
 * (Stems, key) are wbx-67 and later.
 */
import { memo, type ReactNode } from "react";

import type { PhraseSpan } from "@/lib/player";
import styles from "./LargePlayer.module.css";

/** The phrase row above the detail, in CSS pixels (`.pb` in screens/mock.html). */
export const LARGE_PHRASE_H = 14;

export interface LargePlayerProps {
  /** The deck's own element: a pointer down anywhere inside gives the deck the arrows. */
  shell: React.RefObject<HTMLElement | null>;
  armed: boolean;
  empty: boolean;
  /** The compact strip, drawn by `CompactPlayer` for the same deck. */
  strip: ReactNode;
}

export const LargePlayer = memo(function LargePlayer({ shell, armed, empty, strip, children }: LargePlayerProps & { children: ReactNode }) {
  return (
    <section
      ref={shell}
      className={styles.large}
      aria-label="Preview player"
      data-testid="large-player"
      data-armed={armed ? "" : undefined}
      data-empty={empty ? "" : undefined}
      data-own-activation=""
    >
      {strip}
      {children}
    </section>
  );
});

/**
 * The phrases of the part of the track the detail shows, placed in the
 * scrolling layer's own coordinates (`window` is that layer's span of the
 * track, 0 to 1), so they slide with the waveform under the playhead.
 */
export const DetailPhrases = memo(function DetailPhrases({ phrases, window }: {
  phrases: readonly PhraseSpan[];
  window: { from: number; to: number };
}) {
  const width = window.to - window.from;
  if (phrases.length === 0 || width <= 0) return null;
  return (
    <div className={styles.phrases} data-testid="large-player-phrases" aria-label="Phrases">
      {phrases.filter((span) => span.to > window.from && span.from < window.to).map((span) => (
        <span
          key={`${span.from}-${span.label}`}
          className={styles.phrase}
          data-phrase={span.kind}
          style={{ left: `${((span.from - window.from) / width) * 100}%`, width: `${((span.to - span.from) / width) * 100}%` }}
          title={span.label}
        >
          {span.label}
        </span>
      ))}
    </div>
  );
});

export { styles as largePlayerStyles };
