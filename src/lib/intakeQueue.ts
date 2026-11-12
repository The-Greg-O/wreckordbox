/**
 * The Intake view's queue (wbx-62), in the view's terms.
 *
 * Intake (tracks with no Prep colour) and Problems (tracks whose Prep colour
 * is Problem) are queries of the Library service (wbx-61), which the window
 * hosts. The service answers in its own snake_case; `fromServicePage` lifts
 * a page into these types. What a track still needs is the service's to
 * decide; `needsLine` only words it for the queue's rows.
 */
import type { RowDto } from "@/ipc/types";

/** The two queues the view shows: the tree's Intake and Problems. */
export type IntakeQueueName = "intake" | "problems";

/** The service's names for what a track still needs (`intake::needs`). */
export type IntakeNeed = "title" | "artist" | "energy" | "tags" | "hot_cues" | "grid";

/** Whether the DJ has confirmed the beat grid (the Analysis Lock). */
export type IntakeGrid = "not_analysed" | "not_confirmed" | "confirmed";

/** An agent's proposed title or artist, waiting for the DJ. */
export interface IntakeSuggestion {
  field: string;
  value: string;
  /** Who proposed it: the source's name. */
  source: string;
  confidence: number;
}

/** One track in a queue, and what it still needs. */
export interface QueuedTrack {
  id: string;
  title: string;
  artist: string;
  bpm: number | null;
  key: string;
  /** In the service's order; may hold names this view does not know yet. */
  needs: string[];
  /** The dimensions that reach the deck and hold no tag on this track. */
  emptyDimensions: { id: string; name: string }[];
  grid: IntakeGrid;
  suggestions: IntakeSuggestion[];
}

/** A page of a queue: its tracks, and the row a deck loads for each. */
export interface IntakeQueuePage {
  tracks: QueuedTrack[];
  /** `rows[i]` is `tracks[i]` as the deck and the lists know it. */
  rows: RowDto[];
  total: number;
  offset: number;
  /** Where the next page starts; `null` on the last one. */
  nextOffset: number | null;
  /** Why the queue cannot be listed now (Problems with no colour named Problem); `null` when it can. */
  unavailable: string | null;
  /** Why proposed titles and artists could not be read (the Sidecar busy or unreadable); `null` when they were. */
  suggestionsUnavailable: string | null;
}

/** How many tracks wait in each queue. `problems` is `null` when no colour is named Problem. */
export interface IntakeCounts {
  intake: number;
  problems: number | null;
}

/** The app shell's `intake_queue` answer, as it arrives. */
export interface ServiceQueuePage {
  queue: {
    tracks: {
      id: string; title: string; artist: string; bpm: number | null; key: string; energy?: number | null;
      needs: string[];
      empty_dimensions: { id: string; name: string; kind?: string }[];
      grid: IntakeGrid;
      suggestions: { field: string; fact_id?: number; value: string; source: { kind: string; name: string }; confidence: number }[];
    }[];
    total: number;
    offset: number;
    next_offset: number | null;
    suggestions_unavailable?: string | null;
    limit?: number;
    prep_colour?: unknown;
  } | null;
  rows: RowDto[];
  unavailable: { code: string; message: string } | null;
}

export function fromServicePage(raw: ServiceQueuePage): IntakeQueuePage {
  const queue = raw.queue;
  return {
    tracks: (queue?.tracks ?? []).map((track) => ({
      id: track.id,
      title: track.title,
      artist: track.artist,
      bpm: track.bpm,
      key: track.key,
      needs: [...track.needs],
      emptyDimensions: track.empty_dimensions.map(({ id, name }) => ({ id, name })),
      grid: track.grid,
      suggestions: track.suggestions.map(({ field, value, source, confidence }) => ({ field, value, source: source.name, confidence })),
    })),
    rows: raw.rows,
    total: queue?.total ?? 0,
    offset: queue?.offset ?? 0,
    nextOffset: queue?.next_offset ?? null,
    unavailable: raw.unavailable?.message ?? null,
    suggestionsUnavailable: queue?.suggestions_unavailable ?? null,
  };
}

/** What each need is called on a queue row. */
const NEED_WORDS: Record<IntakeNeed, string> = {
  title: "title",
  artist: "artist",
  energy: "Energy",
  tags: "tags",
  hot_cues: "cues",
  grid: "grid",
};

/** The needs that, all missing together, read as "everything". */
const EVERYTHING: readonly IntakeNeed[] = ["energy", "tags", "hot_cues", "grid"];

/**
 * What a queued track still needs, as one line: "needs title · Energy ·
 * cues". A single empty dimension is named ("needs Situation"). It informs;
 * it never blocks anything.
 */
export function needsLine(track: QueuedTrack): string {
  const known = track.needs.filter((need): need is IntakeNeed => need in NEED_WORDS);
  if (known.length === 0) return "nothing missing";
  if (EVERYTHING.every((need) => known.includes(need))) return "needs everything";
  const words = known.map((need) => {
    const only = track.emptyDimensions.length === 1 ? track.emptyDimensions[0] : undefined;
    return need === "tags" && only !== undefined ? only.name : NEED_WORDS[need];
  });
  return `needs ${words.join(" · ")}`;
}

/** The queue's heading count: "2 of 23" with a track chosen, else the total. */
export function queuePosition(index: number, total: number): string {
  return index >= 0 && total > 0 ? `${index + 1} of ${total}` : String(total);
}
