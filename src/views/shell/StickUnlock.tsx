/**
 * Unlocking a Stick (wbx-45.11): a Spare stick with a plain confirmation,
 * a Gig stick only with its name typed, which the backend checks. Shown in
 * the staged write's dialog, the Stick's panel and the Stick Protection
 * dialog the command palette opens. Locking it again is one press.
 */
import { useState } from "react";

import type { StickProtection } from "@/ipc/types";
import { lockStick, unlockStick } from "@/store/useStickProtection";
import styles from "./StickWriteGate.module.css";

export function StickUnlock({ path, protection, onChange }: {
  path: string;
  protection: StickProtection;
  onChange?: (now: StickProtection) => void;
}) {
  const [typed, setTyped] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const act = async (run: () => Promise<StickProtection>) => {
    setBusy(true);
    setError(null);
    try {
      const now = await run();
      setTyped("");
      onChange?.(now);
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally {
      setBusy(false);
    }
  };
  const name = protection.name;
  if (!protection.protected) {
    return (
      <div className={styles.protection} data-stick-protection="unlocked">
        <span>{name} is unlocked until it is ejected or removed.</span>
        <button type="button" disabled={busy} onClick={() => { void act(() => lockStick(path)); }}>Lock {name}</button>
      </div>
    );
  }
  return (
    <form
      className={styles.protection}
      data-stick-protection="protected"
      onSubmit={(event) => {
        event.preventDefault();
        void act(() => unlockStick(path, protection.spare ? undefined : typed));
      }}
    >
      <span>
        {name} is protected: nothing is written to it until it is unlocked.
        {protection.spare ? "" : ` It is a Gig stick: type its name, ${name}, to unlock it.`}
      </span>
      {protection.spare ? null : (
        <input
          aria-label={`Type ${name} to unlock it`}
          value={typed}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => { setTyped(event.target.value); }}
        />
      )}
      <button type="submit" disabled={busy || (!protection.spare && typed.trim() === "")}>Unlock {name}</button>
      {error === null ? null : <span role="alert" className={styles.unlockError}>{error}</span>}
    </form>
  );
}
