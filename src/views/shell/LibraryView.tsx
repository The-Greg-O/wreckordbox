import type { ReactNode } from "react";

/**
 * The Library view: a playlist or All Tracks as a list.
 *
 * For now it is exactly the list the old layout draws, handed in by the
 * window (`list`), so the shell is useful from the first day. The Library's
 * own columns (ticket #55), the Track Filter bar and the second list are
 * built here, around or in place of that list.
 */
export function LibraryView({ list }: { list: ReactNode }) {
  return <>{list}</>;
}
