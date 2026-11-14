/**
 * The running load in the Stick view (wbx-73): the loud display
 * (`LoudLoad.tsx`) or, with the `view.loudStickLoad` setting off, wbx-72's
 * plain bar (`PlainLoadProgress`). Both take the same props, shrink with
 * `Esc` or their button, and offer the switch to the other.
 */
import type { LoadProgress } from "@/lib/holdToLoad";
import type { StickLoad } from "@/store/stickLoads";
import { usePreferences } from "@/store/usePreferences";
import { LoudLoad } from "./LoudLoad";
import { PlainLoadProgress, useEscapeShrinks } from "./StickLoadProgress";

export function LoadProgressPanel({ load, progress }: { load: StickLoad; progress: LoadProgress | null }) {
  useEscapeShrinks(load.id);
  const Display = usePreferences().view.loudStickLoad ? LoudLoad : PlainLoadProgress;
  return <Display load={load} progress={progress} />;
}
