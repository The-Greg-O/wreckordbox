import { useEffect, useMemo, useState } from "react";
import { getBackend } from "@/ipc/client";
import type { FilterValues, TrackFilter, ViewSpec } from "@/ipc/types";
import {
  EMPTY_SHELL_FILTER, dropStale, toViewFilter, type LoadedTrack, type ShellFilter,
} from "@/lib/shellTrackFilter";

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
  /** The keys the open list offers (a Stick's own on a Stick list), or why not. */
  values: FilterValues | null;
  valuesError: string | null;
  /** The Library's MyTag categories, whatever list is open. */
  categories: FilterValues["tags"];
  /** Picks the Library no longer holds, left out of the filter, as the service names them. */
  dropped: string[];
  /** Whether the open list is a Stick's, where MyTags cannot be read. */
  onStick: boolean;
  /** Whether anything narrows the open list now. */
  filtering: boolean;
  count: FilterCount;
}

/** How long a burst of changes (typing, picks) settles before the bar reads. */
const SETTLE_MS = 150;

const COLLECTION: ViewSpec = { source: { kind: "collection" }, sort: "trackNo", descending: false, query: "" };

/**
 * The Library view's Track Filter (wbx-57): the bar's model, what the list is
 * sent, the choices it can offer, and the "n of m" count.
 *
 * - The text is the list's own search (`query` / `setQuery`).
 * - `base` is the list's spec with no query and no filter: "of m" is
 *   everything the open list holds. It is read once per list and Library
 *   state (`generation`, `tagListRevision`), not on every keystroke.
 * - The Library's MyTag categories are read on their own (not the open
 *   list's): a picked tag or category the Library no longer holds is dropped
 *   from the bar and named in `dropped`, as the service drops and names it.
 * - `loaded` is the track Key match and BPM ±6% follow.
 * - On a Stick list MyTag picks are not sent (a Stick's MyTags are not read);
 *   the bar says so.
 *
 * Reads settle for `SETTLE_MS` after a change; an answer for an earlier
 * state is dropped, and a read that fails is shown as failed rather than
 * leaving the last number up.
 */
export function useShellTrackFilter({
  enabled, base, query, setQuery, generation, tagListRevision, loaded,
}: {
  enabled: boolean;
  base: ViewSpec;
  query: string;
  setQuery: (text: string) => void;
  generation: number;
  tagListRevision: number;
  loaded: LoadedTrack;
}): ShellTrackFilter {
  const [picks, setPicks] = useState<ShellFilter>(EMPTY_SHELL_FILTER);
  const model = useMemo(() => ({ ...picks, text: query }), [picks, query]);
  const [values, setValues] = useState<FilterValues | null>(null);
  const [valuesError, setValuesError] = useState<string | null>(null);
  const [library, setLibrary] = useState<FilterValues | null>(null);
  const [dropped, setDropped] = useState<string[]>([]);
  const [total, setTotal] = useState<FilterCount>({ state: "reading" });
  const [narrowed, setNarrowed] = useState<FilterCount>({ state: "reading" });

  const onStick = base.source.kind === "device";
  const categories = useMemo(() => library?.tags ?? [], [library]);
  const categoryNames = useMemo(() => categories.map((c) => c.name), [categories]);
  const loadedKey = loaded.key;
  const loadedBpm = loaded.bpmX100;
  const viewFilter = useMemo(() => {
    // Picks only: the text reaches the list through `query`.
    const filter = toViewFilter({ ...picks, text: "" }, categoryNames, { key: loadedKey, bpmX100: loadedBpm }).filter;
    if (!filter || !onStick || !filter.tags) return filter;
    const { tags: _notOnAStick, ...rest } = filter;
    return Object.keys(rest).length > 0 ? rest : undefined;
  }, [picks, categoryNames, loadedKey, loadedBpm, onStick]);
  // What is actually sent: on a Stick, MyTag picks are not.
  const filtering = query.trim() !== "" || viewFilter !== undefined;

  const baseKey = JSON.stringify(base);
  const filterKey = JSON.stringify(viewFilter ?? null);

  // The Library's categories, for the bar's MyTag controls and stale picks.
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    void (async () => {
      try {
        const answer = await (await getBackend()).filterValues(COLLECTION);
        if (live) setLibrary(answer);
      } catch {
        // Not up yet; the ready event bumps `generation`.
      }
    })();
    return () => { live = false; };
  }, [enabled, generation, tagListRevision]);

  // One stale-pick policy with the service: what the Library no longer
  // holds leaves the filter, and the bar says what left.
  useEffect(() => {
    if (!library) return;
    const { filter, dropped: gone } = dropStale(picks, library.tags);
    if (gone.length === 0) return;
    setPicks(filter);
    setDropped(gone);
  }, [library, picks]);

  // The keys the open list offers, for the list and the search alone.
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    const spec: ViewSpec = { ...(JSON.parse(baseKey) as ViewSpec), query };
    const timer = setTimeout(() => {
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
    }, SETTLE_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [enabled, baseKey, query, generation, tagListRevision]);

  // "of m": the open list unfiltered, once per list and Library state.
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    setTotal({ state: "reading" });
    void (async () => {
      try {
        const all = await (await getBackend()).openView({ ...(JSON.parse(baseKey) as ViewSpec), query: "" });
        if (live) setTotal({ state: "known", matching: all.len, total: all.len });
      } catch (error) {
        if (live) setTotal({ state: "failed", message: error instanceof Error ? error.message : String(error) });
      }
    })();
    return () => { live = false; };
  }, [enabled, baseKey, generation, tagListRevision]);

  // "n": the list as filtered, once a burst of changes has settled.
  useEffect(() => {
    if (!enabled || !filtering) return;
    let live = true;
    setNarrowed({ state: "reading" });
    const spec = JSON.parse(baseKey) as ViewSpec;
    const filter = JSON.parse(filterKey) as TrackFilter | null;
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const view = await (await getBackend()).openView({ ...spec, query, ...(filter ? { filter } : {}) });
          if (live) setNarrowed({ state: "known", matching: view.len, total: view.len });
        } catch (error) {
          if (live) setNarrowed({ state: "failed", message: error instanceof Error ? error.message : String(error) });
        }
      })();
    }, SETTLE_MS);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [enabled, filtering, baseKey, filterKey, query, generation, tagListRevision]);

  const count: FilterCount = total.state !== "known"
    ? total
    : !filtering
      ? total
      : narrowed.state !== "known"
        ? narrowed
        : { state: "known", matching: narrowed.matching, total: total.total };

  const onChange = useMemo(() => (next: ShellFilter) => {
    setPicks({ ...next, text: "" });
    setDropped([]);
    if (next.text !== query) setQuery(next.text);
  }, [query, setQuery]);

  return { model, onChange, viewFilter, values, valuesError, categories, dropped, onStick, filtering, count };
}
