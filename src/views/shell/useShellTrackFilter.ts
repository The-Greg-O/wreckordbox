import { useEffect, useMemo, useState } from "react";
import { getBackend } from "@/ipc/client";
import type { FilterValues, TrackFilter, ViewSpec } from "@/ipc/types";
import { EMPTY_SHELL_FILTER, toViewFilter, type ShellFilter } from "@/lib/shellTrackFilter";

/** What the bar's count shows. */
export type FilterCount =
  | { state: "known"; matching: number; total: number }
  | { state: "reading" }
  | { state: "failed"; message: string };

export interface ShellTrackFilter {
  /** The model, with the list's search text as its `text`. */
  model: ShellFilter;
  onChange: (next: ShellFilter) => void;
  /** What the window's list is sent: `ViewSpec.filter`. */
  viewFilter: TrackFilter | undefined;
  /** The keys and MyTag categories the open list offers, or why not. */
  values: FilterValues | null;
  valuesError: string | null;
  count: FilterCount;
}

/** How long a burst of changes (typing, picks) settles before the count is read. */
const SETTLE_MS = 120;

/**
 * The Library view's Track Filter (wbx-57): the bar's model, what the list is
 * sent, the values it can offer, and the "n of m" count.
 *
 * The text is the list's own search (`query` / `setQuery`), so the bar's
 * text field and the list's search box are one search. `base` is the list's
 * spec with no query and no filter: the count's "of m" is everything the
 * open list holds. Both reads are re-asked when the list, the filter or the
 * Library changes; an answer for an earlier state is dropped, and a read that
 * fails is shown as failed rather than leaving the last number up.
 */
export function useShellTrackFilter({
  enabled, base, query, setQuery, generation,
}: {
  enabled: boolean;
  base: ViewSpec;
  query: string;
  setQuery: (text: string) => void;
  generation: number;
}): ShellTrackFilter {
  const [picks, setPicks] = useState<ShellFilter>(EMPTY_SHELL_FILTER);
  const model = useMemo(() => ({ ...picks, text: query }), [picks, query]);
  const [values, setValues] = useState<FilterValues | null>(null);
  const [valuesError, setValuesError] = useState<string | null>(null);
  const [count, setCount] = useState<FilterCount>({ state: "reading" });

  const categories = useMemo(() => (values?.tags ?? []).map((c) => c.name), [values]);
  // Picks only: the text reaches the list through `query`.
  const viewFilter = useMemo(() => toViewFilter({ ...picks, text: "" }, categories).filter, [picks, categories]);

  const baseKey = JSON.stringify(base);
  const filterKey = JSON.stringify(viewFilter ?? null);

  // The values the bar offers, for the open list and the search alone.
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    const spec: ViewSpec = { ...(JSON.parse(baseKey) as ViewSpec), query };
    void (async () => {
      try {
        const answer = await (await getBackend()).filterValues(spec);
        if (!live) return;
        setValues(answer);
        setValuesError(null);
      } catch (error) {
        if (!live) return;
        setValuesError(error instanceof Error ? error.message : String(error));
      }
    })();
    return () => { live = false; };
  }, [enabled, baseKey, query, generation]);

  // "n of m": both read once a burst of changes has settled.
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    setCount({ state: "reading" });
    const spec = JSON.parse(baseKey) as ViewSpec;
    const filter = JSON.parse(filterKey) as TrackFilter | null;
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const backend = await getBackend();
          const all = await backend.openView({ ...spec, query: "" });
          const narrowed = filter === null && query.trim() === ""
            ? all
            : await backend.openView({ ...spec, query, ...(filter ? { filter } : {}) });
          if (live) setCount({ state: "known", matching: narrowed.len, total: all.len });
        } catch (error) {
          if (live) setCount({ state: "failed", message: error instanceof Error ? error.message : String(error) });
        }
      })();
    }, SETTLE_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [enabled, baseKey, filterKey, query, generation]);

  const onChange = useMemo(() => (next: ShellFilter) => {
    setPicks({ ...next, text: "" });
    if (next.text !== query) setQuery(next.text);
  }, [query, setQuery]);

  return { model, onChange, viewFilter, values, valuesError, count };
}

