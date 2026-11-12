/**
 * A list's look, handed to `TrackTable` by a view that draws rows its own way
 * (the view shell's Library, ticket #55). Without one the table is upstream's.
 *
 * The table keeps all its behaviour (selection, keys, drags, menus, editing);
 * a look changes only what a row and its headings look like, which extra
 * fields are fetched, and what the headings' menu offers.
 */
import type { ReactNode } from "react";

import type { RowDto } from "@/ipc/types";
import type { ColumnKey, ColumnSpec } from "@/lib/columns";

export interface ListLook {
  /** Put on the list's root, for the look's own stylesheet. */
  className: string;
  /** A row's height at 100% line spacing; Browse › Line Space still scales it. */
  rowHeight: number;
  /** The headings' row height. */
  headingHeight: number;
  /** Extra fields fetched whatever columns are on show (the Prep colour stripe needs `color`). */
  fetch: readonly ColumnKey[];
  /** What the headings' menu lists, and which of those cannot be turned off. */
  menu: { columns: readonly ColumnSpec[]; required: readonly ColumnKey[] };
  /**
   * A cell drawn the look's way, or undefined for the table's own. `rate` is
   * the row's rating edit, absent when the list cannot write.
   */
  cell?: (key: ColumnKey, row: RowDto, rate: ((stars: number) => void) | undefined) => ReactNode | undefined;
  /** Drawn first in every row, over its left edge. */
  edge?: (row: RowDto) => ReactNode;
}
