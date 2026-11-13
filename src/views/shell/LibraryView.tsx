import type { ReactNode } from "react";

import { useTranslation } from "@/i18n";
import { runCommand } from "@/lib/commands";
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
export function LibraryView({ list, target, crumb }: { list: ReactNode; target?: ReactNode; crumb?: readonly string[] | undefined }) {
  return (
    <div className={styles.two}>
      <div className={styles.source}>
        {/* Always rendered (drawn only at half width), so the list beside it is never remounted. */}
        <LibraryCrumb crumb={crumb ?? []} />
        {list}
      </div>
      {target ?? null}
    </div>
  );
}

/**
 * At half width the open list's heading (wbx-45.21; docs/design/screens/
 * library "Sizes": "no tree (the playlist name becomes a breadcrumb)"): its
 * folders, then its name. Pressing it is "Go to the Tree": the tree, drawn
 * over the view, to open another list. It owns Space and Enter.
 */
function LibraryCrumb({ crumb }: { crumb: readonly string[] }) {
  const t = useTranslation();
  const path = crumb.join(" / ");
  return (
    <button
      type="button"
      className={styles.crumb}
      data-shell-crumb=""
      data-own-activation=""
      aria-label={`${t("Open the tree")}: ${path}`}
      title={t("Open the tree to choose another list")}
      onClick={() => { runCommand("goToTree"); }}
    >
      {crumb.map((part, at) => (
        <span key={`${at}:${part}`} className={at === crumb.length - 1 ? styles.crumbName : styles.crumbFolder}>
          {at > 0 ? <span className={styles.crumbSep} aria-hidden>/</span> : null}
          <span data-crumb="">{part}</span>
        </span>
      ))}
      <span className={styles.crumbCaret} aria-hidden>▾</span>
    </button>
  );
}
