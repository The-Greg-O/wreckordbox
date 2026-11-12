/**
 * The Library view's column choice (ticket #55), remembered between sessions
 * under its own name, apart from the old layout's tables (`useColumns`).
 *
 * Returns the props the list takes for it, look included, so the window
 * hands them over in one spread.
 */
import { useCallback, useEffect, useMemo, useState } from "react";

import { autoSizeAll, autoSizeColumn, moveColumn, resizeColumn, type ColumnKey, type Layout } from "@/lib/columns";
import {
  LIBRARY_COLUMNS_STORAGE_KEY,
  libraryDefaultLayout,
  resolveLibrary,
  sanitiseLibrary,
  toggleLibraryColumn,
} from "@/lib/libraryColumns";
import { LIBRARY_LOOK } from "@/views/shell/libraryLook";

function load(): Layout {
  try {
    const raw = localStorage.getItem(LIBRARY_COLUMNS_STORAGE_KEY);
    return sanitiseLibrary(raw === null ? null : JSON.parse(raw));
  } catch {
    return libraryDefaultLayout();
  }
}

export function useLibraryColumns() {
  const [layout, setLayout] = useState(load);
  useEffect(() => {
    try {
      localStorage.setItem(LIBRARY_COLUMNS_STORAGE_KEY, JSON.stringify(layout));
    } catch {
      // Not remembering the choice is no reason to break the session.
    }
  }, [layout]);
  const columns = useMemo(() => resolveLibrary(layout), [layout]);
  const onColumnToggle = useCallback((key: ColumnKey) => setLayout((l) => toggleLibraryColumn(l, key)), []);
  const onColumnMove = useCallback((key: ColumnKey, to: number) => setLayout((l) => moveColumn(l, key, to)), []);
  const onColumnResize = useCallback((key: ColumnKey, width: number) => setLayout((l) => resizeColumn(l, key, width)), []);
  const onColumnAutoSize = useCallback((key: ColumnKey) => setLayout((l) => autoSizeColumn(l, key)), []);
  const onColumnAutoSizeAll = useCallback(() => setLayout(autoSizeAll), []);
  return useMemo(() => ({
    columns, onColumnToggle, onColumnMove, onColumnResize, onColumnAutoSize, onColumnAutoSizeAll,
    look: LIBRARY_LOOK,
    // The Library view has one form of row; upstream's whole-artwork rows
    // and their buttons (`onArtworkMode`, not given) stay with the old layout.
    artworkMode: "partial" as const,
  }), [columns, onColumnToggle, onColumnMove, onColumnResize, onColumnAutoSize, onColumnAutoSizeAll]);
}
