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
  /**
   * The keys and MyTags the open list offers (a Stick's own on a Stick
   * list), or why not; `null` until read for the open list.
   */
  values: FilterValues | null;
  valuesError: string | null;
  /** The MyTag categories the bar offers: the Library's, or on a Stick list the Stick's own. */
  categories: FilterValues["tags"];
  /** Picks the Library no longer holds, left out of the filter, as the service names them. */
  dropped: string[];
  /** Whether the open list is a Stick's, narrowed by the Stick's own MyTags. */
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
 * - On a Stick list the categories are the Stick's own (wbx-45.12), read with
 *   the list's choices; a pick is sent by its category's place among them,
 *   and a pick the Stick does not hold is left out (the bar names it) but
 *   kept for the Library. Until the Stick's categories are read for this
 *   list, the count reads; if they cannot be read, it says why.
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
  const [answered, setValues] = useState<{ base: string; values: FilterValues } | null>(null);
  const [valuesError, setValuesError] = useState<string | null>(null);
  const [library, setLibrary] = useState<FilterValues | null>(null);
  const [dropped, setDropped] = useState<string[]>([]);
  const [total, setTotal] = useState<FilterCount>({ state: "reading" });
  const [narrowed, setNarrowed] = useState<FilterCount>({ state: "reading" });

  const onStick = base.source.kind === "device";
  const baseKey = JSON.stringify(base);
  // Choices read for another list are not this list's.
  const values = answered?.base === baseKey ? answered.values : null;
  // A Stick's own categories, once read for this list (null until then).
  const stickTags = onStick && values !== null && valuesError === null ? values.tags : null;
  const categories = useMemo(() => (onStick ? stickTags ?? [] : library?.tags ?? []), [onStick, stickTags, library]);
  const categoryNames = useMemo(() => categories.map((c) => c.name), [categories]);
  const loadedKey = loaded.key;
  const loadedBpm = loaded.bpmX100;
  // On a Stick, the picks it holds; the rest wait for the Library.
  const tagsPending = onStick && stickTags === null && picks.myTags.length > 0;
  const viewFilter = useMemo(() => {
    const sent = !onStick ? picks : stickTags === null ? { ...picks, myTags: [] } : dropStale(picks, stickTags).filter;
    // Picks only: the text reaches the list through `query`.
    return toViewFilter({ ...sent, text: "" }, categoryNames, { key: loadedKey, bpmX100: loadedBpm }).filter;
  }, [picks, categoryNames, loadedKey, loadedBpm, onStick, stickTags]);
  const filtering = query.trim() !== "" || viewFilter !== undefined;

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
  // On a Stick list the Stick's categories decide what is sent, and the
  // Library's picks are kept for when a Library list is open again.
  useEffect(() => {
    if (!library || onStick) return;
    const { filter, dropped: gone } = dropStale(picks, library.tags);
    if (gone.length === 0) return;
    setPicks(filter);
    setDropped(gone);
  }, [library, picks, onStick]);

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
          setValues({ base: baseKey, values: answer });
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

  const count: FilterCount = tagsPending && valuesError !== null
    ? { state: "failed", message: `this Stick's MyTags could not be read: ${valuesError}` }
    : total.state !== "known"
    ? total
    : tagsPending
      ? { state: "reading" }
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
