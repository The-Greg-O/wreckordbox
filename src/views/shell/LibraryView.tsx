import type { ReactNode } from "react";

import styles from "./LibraryView.module.css";

/**
 * The Library view: a playlist or All Tracks as a list.
 *
 * The list is the one the window builds (`list`: upstream's list with the
 * Library's columns and the Track Filter bar). Beside it, when the DJ shows
 * it, the second list (`target`, wbx-59: `TargetList.tsx`).
 *
 * The list sits in the same place in the page whether the second list is
 * shown or not, so showing, hiding or narrowing the window never remounts
 * it: its selection, filter and scroll stay, and the window's copy of its
 * selection never outlives what it shows.
 */
export function LibraryView({ list, target }: { list: ReactNode; target?: ReactNode }) {
  return (
    <div className={styles.two}>
      <div className={styles.source}>{list}</div>
      {target ?? null}
    </div>
  );
}
