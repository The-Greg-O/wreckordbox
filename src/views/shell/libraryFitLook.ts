/**
 * The Library look at the width the shell has and with or without the
 * second list (wbx-60): `LIBRARY_LOOK` with a `fit` that draws what
 * `fitLibraryColumns` keeps. One object per case, so the list sees a new
 * look only when the case changes.
 */
import { fitLibraryColumns, type LibraryWidth } from "@/lib/libraryFit";
import type { ListLook } from "@/views/browser/listLook";
import { LIBRARY_LOOK } from "./libraryLook";

const looks = new Map<string, ListLook>();

export function libraryLookFor(width: LibraryWidth, twoLists: boolean): ListLook {
  const id = `${width}:${twoLists}`;
  let look = looks.get(id);
  if (!look) {
    look = { ...LIBRARY_LOOK, fit: (drawn, available) => fitLibraryColumns(drawn, available, { width, twoLists }) };
    looks.set(id, look);
  }
  return look;
}
