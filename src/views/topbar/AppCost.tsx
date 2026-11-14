/**
 * What the app is costing, in the title bar's corner.
 *
 * Its own component so that a reading re-renders these figures and
 * nothing else: as a value of the shell it re-rendered the whole window
 * every time, which was most of the webview's idle load.
 *
 * A dash is a figure the platform will not give rather than a zero, which
 * would be a claim. GPU is not among them: macOS accounts it per process
 * only to root, so it was always a dash.
 *
 * UNDERRUNS is the device's own count of audio actually lost (wbx-45.20),
 * beside AUDIO's late callbacks; the latency is in its accessible name.
 */
import { formatCount, formatLatency, formatMemory, formatPercent, useAppCost } from "@/store/useDiagnostics";
import { useTooltip } from "@/store/usePreferences";

import costStyles from "./AppCost.module.css";

export function AppCost({ className }: { className?: string | undefined }) {
  const cost = useAppCost();
  const tip = useTooltip();
  return (
    <div className={className} data-testid="app-cost">
      <span title={tip(`Audio callback deadline usage; ${cost.audioXruns} overruns`)}>AUDIO {formatPercent(cost.audioLoad * 100)}</span>
      {/* The device's own underruns: audio actually lost, heard as a click.
          Marked once there are any, so a stutter shows without the log. */}
      <span
        data-testid="app-cost-underruns"
        className={costStyles.underruns}
        data-lost={cost.audioUnderruns > 0 ? "true" : undefined}
        aria-label={`Device underruns: ${cost.audioUnderruns}; output latency ${formatLatency(cost.audioLatencyMs)}`}
        title={tip(`Underruns the audio device reported since playback started; output latency ${formatLatency(cost.audioLatencyMs)}`)}
      >UNDERRUNS {cost.audioUnderruns}</span>
      <span title={tip("Resident memory")}>RAM {formatMemory(cost.memoryMb)}</span>
      <span title={tip("Frames a second, timed in the window")}>FPS {formatCount(cost.fps)}</span>
    </div>
  );
}
