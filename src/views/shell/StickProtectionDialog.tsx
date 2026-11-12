/**
 * Stick Protection (wbx-45.11), from the command palette or the status
 * line: every mounted Stick, protected or unlocked, with its unlock (a
 * Spare stick's confirmation, a Gig stick's typed name) or its lock.
 */
import { useEffect, useRef } from "react";

import type { Device, StickProtection } from "@/ipc/types";
import styles from "./StickWriteGate.module.css";
import { StickUnlock } from "./StickUnlock";

export function StickProtectionDialog({ devices, protection, onClose }: {
  devices: readonly Device[];
  protection: ReadonlyMap<string, StickProtection>;
  onClose: () => void;
}) {
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => { close.current?.focus(); }, []);
  return (
    <div className={styles.backdrop}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="stick-protection-title"
        className={styles.dialog}
        data-testid="stick-protection"
        onKeyDown={(event) => {
          event.stopPropagation();
          if (event.key === "Escape") onClose();
        }}
      >
        <h2 id="stick-protection-title" className={styles.title}>Stick Protection</h2>
        <p className={styles.note}>
          Every Stick is protected until it is unlocked: nothing is written to it, from here, the Library service
          or the command line. An unlock ends when the Stick is ejected, removed or put back.
        </p>
        {devices.length === 0 ? <p className={styles.note}>No Stick is mounted.</p> : devices.map((device) => {
          const found = protection.get(device.path);
          return (
            <section key={device.path} aria-label={device.name}>
              <h3 className={styles.what}>{device.name}</h3>
              {found === undefined ? <p className={styles.note}>Not read yet.</p> : <StickUnlock path={device.path} protection={found} />}
            </section>
          );
        })}
        <div className={styles.buttons}>
          <button ref={close} type="button" onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}
