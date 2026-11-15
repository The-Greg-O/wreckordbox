/**
 * The unfinished copies a refused backup left on a Stick, offered for
 * removal (wbx-45.25).
 *
 * A backup stopped because its Stick was locked, ejected or replaced
 * mid-copy writes nothing more and cleans nothing up. Once that Stick is
 * back, this offers to remove exactly the copies the backup recorded, and
 * nothing else in the folder. The removal is a Stick write like any other:
 * staged through the window's gate, with the Stick unlocked in its dialog
 * and the deliberate hold; the backend refuses it on a protected Stick or
 * one replaced since the offer.
 */
import { useCallback, useEffect, useState } from "react";

import { useTranslation } from "@/i18n";
import { getBackend } from "@/ipc/client";
import type { BackupLeftoverOffer, StickWritePlan } from "@/ipc/types";
import { errorMessage } from "@/lib/errorMessage";
import { useStickWriteGateContext } from "@/views/shell/StickWriteGate";
import { Button } from "./controls";
import styles from "./BackupsPane.module.css";
import layout from "./PaneLayout.module.css";

/** The offer as the gate's already-made plan: the hold approves exactly its id. */
function planned(offer: BackupLeftoverOffer): StickWritePlan {
  return {
    onStick: true, stickName: offer.stickName, protected: offer.protected, spare: false, pending: false,
    audioEstimated: false, tracksNew: 0, tracksChanged: 0, audioReused: 0, tracksRemoved: 0, tracksMissing: 0,
    trackChanges: [], analysisWritten: 0, playlists: [], netAdditionalBytes: 0, adding: null, plan: offer.plan,
  };
}

/** `refreshKey`: read the offer again when it changes (a backup just ended). */
export function BackupLeftovers({ refreshKey, disabled }: { refreshKey: string; disabled: boolean }) {
  const t = useTranslation();
  const gate = useStickWriteGateContext();
  const [offer, setOffer] = useState<BackupLeftoverOffer | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");

  const read = useCallback(async () => {
    const next = await (await getBackend()).backupLeftovers();
    setOffer(next);
    return next;
  }, []);
  useEffect(() => {
    let live = true;
    void getBackend().then(b => b.backupLeftovers()).then(next => { if (live) setOffer(next); })
      .catch(e => { if (live) setError(errorMessage(e)); });
    return () => { live = false; };
  }, [refreshKey]);

  const remove = async () => {
    if (busy) return;
    setBusy(true); setError(""); setMessage("");
    try {
      // Offered afresh: the plan is bound to what is there now.
      const now = await read();
      if (now.leftovers.length === 0 || now.plan === null) return;
      if (gate === null) {
        setError(t("Remove these from the main window: nothing was removed."));
        return;
      }
      const backend = await getBackend();
      let removed: string[] = [];
      const done = await gate.stage({
        path: now.stickRoot,
        title: t("Remove unfinished backups from {name}", { name: now.stickName }),
        lines: now.leftovers.map(name => t("Remove “{name}” (unfinished backup)", { name })),
        plan: { kind: "sweepBackupLeftovers", names: [...now.leftovers] },
        planned: planned(now),
        run: async (approval) => { removed = await backend.sweepBackupLeftovers(approval); },
      });
      if (done) setMessage(t("Removed {count} unfinished backup copies from {name}.", { count: removed.length, name: now.stickName }));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
      await read().catch(e => setError(errorMessage(e)));
    }
  };

  if (offer === null || (offer.leftovers.length === 0 && error === "" && message === "")) return null;
  return <section className={styles.destination} aria-label={t("Unfinished backups")}>
    <div>
      <strong>{t("Unfinished backups")}</strong>
      {error ? <p role="alert" className={styles.error}>{error}</p> : null}
      {message ? <p className={styles.status} role="status">{message}</p> : null}
      {offer.leftovers.length > 0 ? <>
        <p className={layout.help}>{t("A backup to {name} stopped before it finished and left these unfinished copies in its folder. Only these are removed; nothing else on the Stick is touched.", { name: offer.stickName })}</p>
        <ul aria-label={t("Unfinished backup copies")}>{offer.leftovers.map(name => <li key={name}>{name}</li>)}</ul>
        {offer.protected ? <p className={layout.help}>{t("{name} is protected: unlock it to remove them.", { name: offer.stickName })}</p> : null}
      </> : null}
    </div>
    {offer.leftovers.length > 0 ? <div data-own-activation="">
      <Button className={styles.backupButton} disabled={disabled || busy} onClick={() => void remove()}>{t("Remove unfinished copies…")}</Button>
    </div> : null}
  </section>;
}
