/**
 * Cue and grid writes that failed for tracks other than the one shown
 * (wbx-64 review 3): each stays with the track it came from, listed beside
 * the Intake queue as the Tag panel's failures are, until a later write to
 * that track lands or the DJ dismisses it.
 */
import { useEffect, useState } from "react";

import { useTranslation } from "@/i18n";
import { deckWrites, type TrackWriteFailure } from "@/lib/trackWrites";
import styles from "./IntakeTagPanel.module.css";

export function DeckWriteFailures({ titleOf, shownTrack }: { titleOf: (id: string) => string; shownTrack: string | null }) {
  const t = useTranslation();
  const [failures, setFailures] = useState<TrackWriteFailure[]>(() => deckWrites.failures());
  useEffect(() => {
    setFailures(deckWrites.failures());
    return deckWrites.subscribe(() => setFailures(deckWrites.failures()));
  }, []);
  const others = failures.filter((failure) => failure.track !== shownTrack);
  if (others.length === 0) return null;
  return (
    <ul className={styles.failures} data-testid="intake-deck-failures" role="alert" data-own-activation="">
      {others.map((failure) => (
        <li key={`${failure.track}:${failure.kind}`} data-track={failure.track}>
          {failure.kind === "cues" ? t("Cues not saved") : t("Beat grid not saved")} — “{titleOf(failure.track)}”: {failure.message}{" "}
          <button type="button" className={styles.small} onClick={() => deckWrites.dismiss(failure.track, failure.kind)}>{t("Dismiss")}</button>
        </li>
      ))}
    </ul>
  );
}
