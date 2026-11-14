import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

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
export function LibraryView({ list, target, crumb, tally }: {
  list: ReactNode; target?: ReactNode; crumb?: readonly string[] | undefined;
  /** The open playlist's count and length ("30 · 2h24m", wbx-6wz), or "" for a list with no tally. */
  tally?: string | undefined;
}) {
  return (
    <div className={styles.two}>
      <div className={styles.source}>
        {/* Always rendered (drawn only at half width), so the list beside it is never remounted. */}
        <LibraryCrumb crumb={crumb ?? []} tally={tally ?? ""} />
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
 *
 * After the name, the open playlist's count and length (wbx-6wz). It gives
 * way first: it shrinks before the name does (CSS) and, once it no longer
 * fits whole, is not drawn at all rather than cut short (`useWhole`).
 */
function LibraryCrumb({ crumb, tally }: { crumb: readonly string[]; tally: string }) {
  const t = useTranslation();
  const path = crumb.join(" / ");
  const [tallyRef, whole] = useWhole(tally);
  return (
    <button
      type="button"
      className={styles.crumb}
      data-shell-crumb=""
      data-own-activation=""
      aria-label={`${t("Open the tree")}: ${path}${tally ? `, ${tally}` : ""}`}
      title={t("Open the tree to choose another list")}
      onClick={() => { runCommand("goToTree"); }}
    >
      {crumb.map((part, at) => (
        <span key={`${at}:${part}`} className={at === crumb.length - 1 ? styles.crumbName : styles.crumbFolder}>
          {at > 0 ? <span className={styles.crumbSep} aria-hidden>/</span> : null}
          <span data-crumb="">{part}</span>
        </span>
      ))}
      {tally ? (
        <span ref={tallyRef} className={styles.crumbTally} data-crumb-tally="" data-cut={whole ? undefined : ""} aria-hidden>
          {tally}
        </span>
      ) : null}
      <span className={styles.crumbCaret} data-crumb-caret="" aria-hidden>▾</span>
    </button>
  );
}

/**
 * Whether an element shows all of its text: false once it is squeezed
 * narrower than its content. Rechecked on each resize of the element (the
 * window, the scale, the name beside it) and when the text changes.
 */
function useWhole(text: string): [(element: HTMLElement | null) => void, boolean] {
  const [element, setElement] = useState<HTMLElement | null>(null);
  const [whole, setWhole] = useState(true);
  const last = useRef(true);
  useLayoutEffect(() => {
    if (!element) return;
    const check = () => {
      const fits = element.scrollWidth <= element.clientWidth + 0.5;
      if (fits !== last.current) { last.current = fits; setWhole(fits); }
    };
    check();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(check);
    observer.observe(element);
    return () => { observer.disconnect(); };
  }, [element, text]);
  return [setElement, whole];
}
