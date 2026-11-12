/**
 * About: what this is, which version, what it is based on, and under what
 * terms. Not a pane rekordbox has — its version is under the application
 * menu. Everything here is plain text: nothing in it leaves the machine.
 */
import { useEffect, useState } from "react";

import { getBackend } from "@/ipc/client";
import styles from "./Preferences.module.css";

export function AboutPane() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    void getBackend()
      .then((backend) => backend.appVersion())
      .then((found) => {
        if (live) setVersion(found);
      })
      .catch(() => {
        // A build with no shell behind it has no version to give; the pane
        // shows a dash rather than an error nobody asked about.
      });
    return () => {
      live = false;
    };
  }, []);

  return (
    <>
      <section className={styles.section} aria-label="About">
        <div className={styles.aboutHeader}>
          <div>
            <h1 className={styles.aboutName}>wreckordbox</h1>
            <p className={styles.aboutVersion} data-testid="about-version">v{version ?? "—"}</p>
          </div>
        </div>
        <div className={styles.aboutCredit}>
          <p>wreckordbox is based on rbxport.</p>
          <p>Copyright © Chris Le and the rbxport contributors.</p>
        </div>
      </section>
      <div className={styles.aboutLegal}>
        <details>
          <summary>Licence</summary>
          <p className={styles.aboutLine}>
            wreckordbox is free software, licensed under the GNU General Public License version 2 or, at your
            option, any later version. It comes with no warranty. It includes the Rubber Band Library
            by Particular Programs Ltd., under the same licence.
          </p>
        </details>
        <details>
          <summary>Disclaimer</summary>
          <p className={styles.aboutLine}>
            wreckordbox is an independent project and is not affiliated with, endorsed by, or sponsored by
            AlphaTheta Corporation, Pioneer DJ, or any other third party. rekordbox, CDJ, XDJ and
            PRO DJ LINK are trademarks of their respective owners. Back up your library before using
            any feature that writes to it.
          </p>
        </details>
      </div>
    </>
  );
}
