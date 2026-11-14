/**
 * Underruns the audio device reported since the stream opened (wbx-45.20):
 * audio actually lost, heard as a click or a gap, whatever the cause. The
 * audio meter beside it sums late callbacks, which are the app's side; this
 * is the device's.
 *
 * In the old layout's top bar and the shell's status line, because on Linux
 * the title bar (and the readout in it) is not drawn. Not the shell's top
 * bar: at half width it has no room left, and the status line's text gives
 * way instead. It reads the shared readout store, which polls
 * every few seconds, so a burst of underruns arrives as one new count and
 * nothing here runs on the audio thread: the engine only adds to an atomic.
 * Marked in the error colour once there are any. In a narrow shell a zero is
 * not drawn and a count shrinks to a warning sign and the number; the
 * accessible name always says what it is.
 */
import { formatLatency, useAppCost } from "@/store/useDiagnostics";
import { useTooltip } from "@/store/usePreferences";

import styles from "./UnderrunCount.module.css";

export function UnderrunCount() {
  const cost = useAppCost();
  const tip = useTooltip();
  const latency = formatLatency(cost.audioLatencyMs);
  return (
    <span
      className={styles.count}
      role="status"
      data-testid="underrun-count"
      data-lost={cost.audioUnderruns > 0 ? "true" : undefined}
      data-zero={cost.audioUnderruns === 0 ? "true" : undefined}
      aria-label={`Device underruns: ${cost.audioUnderruns}; output latency ${latency}`}
      title={tip(`Underruns the audio device reported since playback started; output latency ${latency}`)}
    >
      <span className={styles.word}>UNDERRUNS </span>
      <span className={styles.mark} aria-hidden="true">⚠ </span>
      {cost.audioUnderruns}
    </span>
  );
}
