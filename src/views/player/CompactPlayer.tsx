/**
 * The compact player (ticket #51): the view shell's deck, the same strip in
 * every view. `docs/design/screens/library/library-1280.png` is the render.
 *
 * Left to right: CUE and PLAY; the title over one line of artist, tempo,
 * key and position; the overview waveform with its hot cue markers and a
 * phrase strip under it; beat jump back, the size selector (4, 8, 16, 32)
 * and beat jump forward.
 *
 * Presentational, as `SimplePlayer` is. `Player` owns the deck (the engine,
 * the cues, the phrases, the frame loop that moves the head, the keys) and
 * hands this the pieces it draws; the jump buttons call the same function as
 * the deck's Jump Forward and Jump Reverse keys, and the selector writes the
 * stored size those keys read. The large player Intake needs (#62) is built
 * on the same deck: see the notes file `compact-player.md`.
 */
import { memo, useCallback, useSyncExternalStore } from "react";

import type { Cue, DeckId, RowDto } from "@/ipc/types";
import { COMPACT_JUMP_BEATS, compactLength, compactPosition, type CompactJumpBeats } from "@/lib/compactPlayer";
import { formatBpm } from "@/lib/format";
import type { PhraseSpan } from "@/lib/player";
import type { DeckLoop } from "@/store/usePlayback";
import { CueMarkers } from "./Player";
import type { PositionSource } from "./TimeReadouts";
import { WaveformDetail } from "./WaveformDetail";
import styles from "./CompactPlayer.module.css";

export interface CompactPlayerProps {
  track: RowDto | null;
  deck: DeckId;
  /** The deck's own element: a pointer down inside it gives the deck the arrows. */
  shell: React.RefObject<HTMLElement | null>;
  armed: boolean;
  droppable: boolean;
  onDragOver: (event: React.DragEvent<HTMLElement>) => void;
  onDrop: (event: React.DragEvent<HTMLElement>) => void;
  playing: boolean;
  idle: boolean;
  onToggle: () => void;
  /** CUE is held, not clicked: on the cue point it previews while down. */
  onCueDown: () => void;
  onCueUp: () => void;
  positionSource: PositionSource;
  /** Seconds in the track, or 0 while nothing is loaded. */
  total: number;
  /** The tempo the deck is playing at, ×100. Zero when the track has none. */
  bpmX100: number;
  /** The key as the DJ chose to read keys (classic or Camelot). */
  keyText: string;
  cues: readonly Cue[];
  /** The deck's own loop, drawn as a band over the overview; lit while it plays. */
  loop: DeckLoop | null;
  /** The phrase strip's blocks, empty when the track has no phrase analysis. */
  phrases: readonly PhraseSpan[];
  overviewRef: React.Ref<HTMLDivElement>;
  overview: { width: number; height: number };
  /** Written by the frame loop, not rendered. */
  overviewHead: React.RefObject<HTMLSpanElement | null>;
  onScrubStart: (event: React.PointerEvent<HTMLDivElement>) => void;
  onScrubMove: (event: React.PointerEvent<HTMLDivElement>) => void;
  onScrubEnd: (event: React.PointerEvent<HTMLDivElement>) => void;
  /** Whether the track has a waveform to draw, finished or being drawn. */
  drawn: boolean;
  jumpBeats: CompactJumpBeats;
  onJumpBeats: (beats: CompactJumpBeats) => void;
  /** One press of beat jump: -1 back, 1 forward, by `jumpBeats`. */
  onJump: (direction: -1 | 1) => void;
}

/** The keys that press a focused button. */
const isActivation = (key: string) => key === " " || key === "Enter";

/** Only the readout subscribes to the head's tenths, not the strip around it. */
const Position = memo(function Position({ source, total }: { source: PositionSource; total: number }) {
  const snapshot = useCallback(() => Math.trunc(source.positionRef.current * 10), [source.positionRef]);
  const tenths = useSyncExternalStore(source.subscribe, snapshot);
  return (
    <span className={styles.position} data-testid="compact-player-position">
      <span data-testid="compact-player-elapsed">{compactPosition(tenths / 10)}</span>
      {" / "}
      <span className={styles.length}>{compactLength(total)}</span>
    </span>
  );
});

export const CompactPlayer = memo(function CompactPlayer({
  track, deck, shell, armed, droppable, onDragOver, onDrop,
  playing, idle, onToggle, onCueDown, onCueUp, positionSource, total, bpmX100, keyText,
  cues, loop, phrases, overviewRef, overview, overviewHead, onScrubStart, onScrubMove, onScrubEnd, drawn,
  jumpBeats, onJumpBeats, onJump,
}: CompactPlayerProps) {
  // The selector is a radio group: the arrows inside it move the choice and
  // must not reach the deck's own arrow keys, which jump.
  const sizeKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1
      : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
    if (step === 0) return;
    event.preventDefault();
    event.stopPropagation();
    const at = COMPACT_JUMP_BEATS.indexOf(jumpBeats);
    const next = COMPACT_JUMP_BEATS[Math.min(Math.max(at + step, 0), COMPACT_JUMP_BEATS.length - 1)];
    if (next !== undefined) onJumpBeats(next);
    const group = event.currentTarget;
    queueMicrotask(() => group.querySelector<HTMLElement>('[aria-checked="true"]')?.focus());
  };

  return (
    <section
      ref={shell}
      className={styles.strip}
      aria-label={deck === "b" ? "Preview player B" : "Preview player"}
      data-testid="compact-player"
      data-armed={armed ? "" : undefined}
      data-empty={track ? undefined : ""}
      data-droppable={droppable || undefined}
      // Space and Enter on a control here press that control (or hold CUE),
      // not the deck's Play/Pause and Load: see OWN_ACTIVATION.
      data-own-activation=""
      onDragOver={onDragOver}
      onDrop={onDrop}
    >
      <div className={styles.transport} role="group" aria-label="Transport">
        <button
          type="button"
          className={styles.button}
          aria-label="Cue"
          onPointerDown={onCueDown}
          onPointerUp={onCueUp}
          onPointerCancel={onCueUp}
          onPointerLeave={onCueUp}
          onKeyDown={(event) => {
            if (!isActivation(event.key)) return;
            event.preventDefault();
            if (!event.repeat) onCueDown();
          }}
          onKeyUp={(event) => {
            if (!isActivation(event.key)) return;
            event.preventDefault();
            onCueUp();
          }}
          onBlur={onCueUp}
          disabled={idle}
        >
          CUE
        </button>
        <button
          type="button"
          className={`${styles.button} ${styles.play}`}
          aria-label={playing ? "Pause" : "Play"}
          data-on={playing ? "" : undefined}
          onClick={onToggle}
          disabled={idle}
        >
          {playing ? "❚❚" : "▶"}
        </button>
      </div>

      <div className={styles.meta}>
        <b className={styles.title} data-testid="compact-player-title">{track ? track.title : ""}</b>
        {track ? (
          <span className={styles.line}>
            <span className={styles.artist} data-testid="compact-player-artist">{track.artist}</span>
            <span className={styles.dot} aria-hidden>·</span>
            <span data-testid="compact-player-tempo">{bpmX100 > 0 ? formatBpm(bpmX100) : "--"}</span>
            <span className={styles.dot} aria-hidden>·</span>
            <span data-testid="compact-player-key">{keyText}</span>
            <span className={styles.dot} aria-hidden>·</span>
            <Position source={positionSource} total={total} />
          </span>
        ) : (
          <span className={styles.line}>No track loaded</span>
        )}
      </div>

      <div
        className={styles.band}
        onPointerDown={onScrubStart}
        onPointerMove={onScrubMove}
        onPointerUp={onScrubEnd}
        onPointerCancel={onScrubEnd}
      >
        <div
          ref={overviewRef}
          className={styles.overview}
          data-testid="compact-player-overview"
          role="progressbar"
          aria-label="Position"
          aria-valuemin={0}
          aria-valuemax={Math.round(total)}
          aria-valuenow={Math.round(positionSource.positionRef.current)}
        >
          {track && drawn ? (
            <WaveformDetail
              trackId={track.id}
              progress={0.5}
              span={1}
              width={overview.width}
              height={overview.height}
              half
            />
          ) : null}
          <CueMarkers cues={cues} totalMs={total * 1000} loop={loop} />
        </div>
        {track && phrases.length > 0 ? (
          <div className={styles.phrases} data-testid="compact-player-phrases" aria-label="Phrases">
            {phrases.map((span) => (
              <span
                key={`${span.from}-${span.label}`}
                className={styles.phrase}
                data-phrase={span.kind}
                style={{ left: `${span.from * 100}%`, width: `${(span.to - span.from) * 100}%` }}
                title={span.label}
              >
                {span.label}
              </span>
            ))}
          </div>
        ) : null}
        <span ref={overviewHead} className={styles.playhead} data-testid="compact-player-head" aria-hidden />
      </div>

      <div className={styles.jump} role="group" aria-label="Beat jump">
        <button
          type="button"
          className={styles.button}
          aria-label="Jump back"
          title={`Jump back ${jumpBeats} beats`}
          onClick={() => onJump(-1)}
          disabled={idle}
        >
          ◀
        </button>
        <div className={styles.sizes} role="radiogroup" aria-label="Beat jump size" onKeyDown={sizeKey}>
          {COMPACT_JUMP_BEATS.map((beats) => (
            <button
              key={beats}
              type="button"
              role="radio"
              className={styles.size}
              aria-checked={beats === jumpBeats}
              aria-label={`${beats} beats`}
              tabIndex={beats === jumpBeats ? 0 : -1}
              onClick={() => onJumpBeats(beats)}
            >
              {beats}
            </button>
          ))}
        </div>
        <button
          type="button"
          className={styles.button}
          aria-label="Jump forward"
          title={`Jump forward ${jumpBeats} beats`}
          onClick={() => onJump(1)}
          disabled={idle}
        >
          ▶
        </button>
      </div>
    </section>
  );
});
