import type { ReactNode } from "react";

import styles from "./LibraryView.module.css";

/**
 * The Library view: a playlist or All Tracks as a list.
 *
 * The list is the one the window builds (`list`: upstream's list with the
 * Library's columns and the Track Filter bar). Beside it, when the DJ shows
 * it, the second list (`target`, wbx-59: `TargetList.tsx`).
 */
export function LibraryView({ list, target }: { list: ReactNode; target?: ReactNode }) {
  if (target === undefined || target === null || target === false) return <>{list}</>;
  return (
    <div className={styles.two}>
      <div className={styles.source}>{list}</div>
      {target}
    </div>
  );
}
