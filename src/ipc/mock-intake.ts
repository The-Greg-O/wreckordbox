/**
 * The mock backend's Intake and Problems queues (wbx-62), answered from its
 * own rows the way the Library service answers them from the index: Intake
 * is every track with no Prep colour, Problems every track whose colour is
 * the one named Problem (Red in the mock, the DJ's own naming), in the
 * collection's order. What a track still needs follows `intake::needs`.
 *
 * URL flags for the browser tests:
 * - `?intake=empty`: every track starts with a Prep colour (Clean, Green),
 *   so the Intake queue is empty; Problems keeps its tracks.
 * - `?noProblemColour`: no colour is named Problem, so Problems cannot be
 *   listed, as with a Library whose labels were never named.
 */
import type { RowDto } from "./types";
import type { IntakeCounts, IntakeQueueName, IntakeQueuePage, QueuedTrack } from "@/lib/intakeQueue";

/** The mock's Problem colour: Red, `ColorID` 2. */
export const MOCK_PROBLEM_COLOUR = 2;
/** The mock's Clean colour: Green, `ColorID` 5. */
export const MOCK_CLEAN_COLOUR = 5;

const NO_PROBLEM_COLOUR = "No colour label is named Problem. Name one in rekordbox's colour labels, or set WRECKORDBOX_PROBLEM_LABEL.";

export interface MockIntakeSource {
  all: readonly RowDto[];
  /** Each row's `ColorID`, by position in `all`; edited in place by the mock. */
  colors: Uint8Array;
  /** The MyTag ids a track carries. */
  tagsOf: (row: RowDto) => readonly string[];
  /** The MyTag categories and their tags; a category with tags reaches the deck. */
  categories: readonly { id: string; name: string; tags: readonly { id: string }[] }[];
  /** Whether the DJ has locked (confirmed) a track's beat grid. */
  gridLocked: (trackId: string) => boolean;
  /** As the mock answers everything: after a short wait. */
  wait: <T>(value: T) => Promise<T>;
}

function readUrl(): URLSearchParams {
  return typeof location === "undefined" ? new URLSearchParams() : new URLSearchParams(location.search);
}

export function createMockIntake(source: MockIntakeSource) {
  const url = readUrl();
  const problemNamed = !url.has("noProblemColour");
  if (url.get("intake") === "empty") {
    source.colors.forEach((colour, at) => { if (colour === 0) source.colors[at] = MOCK_CLEAN_COLOUR; });
  }

  const positions = (queue: IntakeQueueName): number[] => {
    const wanted = queue === "intake" ? 0 : MOCK_PROBLEM_COLOUR;
    const out: number[] = [];
    source.all.forEach((_, at) => { if ((source.colors[at] ?? 0) === wanted) out.push(at); });
    return out;
  };

  const queued = (row: RowDto): QueuedTrack => {
    const held = source.tagsOf(row);
    const emptyDimensions = source.categories
      .filter((category) => category.tags.length > 0 && !category.tags.some((tag) => held.includes(tag.id)))
      .map(({ id, name }) => ({ id, name }));
    const grid = row.analysed === 0 ? "not_analysed" as const : source.gridLocked(row.id) ? "confirmed" as const : "not_confirmed" as const;
    const needs: string[] = [];
    if (row.rating === 0) needs.push("energy");
    if (emptyDimensions.length > 0) needs.push("tags");
    if (row.hotCues.length === 0) needs.push("hot_cues");
    if (grid !== "confirmed") needs.push("grid");
    return {
      id: row.id,
      title: row.title,
      artist: row.artist,
      bpm: row.bpmX100 > 0 ? row.bpmX100 / 100 : null,
      key: row.key,
      needs,
      emptyDimensions,
      grid,
      suggestions: [],
    };
  };

  const intakeQueue = (queue: IntakeQueueName, offset: number, limit: number): Promise<IntakeQueuePage> => {
    if (queue === "problems" && !problemNamed) {
      return source.wait({ tracks: [], rows: [], total: 0, offset, nextOffset: null, unavailable: NO_PROBLEM_COLOUR });
    }
    const all = positions(queue);
    const page = all.slice(offset, offset + limit).map((at) => source.all[at]).filter((row): row is RowDto => row !== undefined);
    const next = offset + page.length;
    return source.wait({
      tracks: page.map(queued),
      rows: page.map((row, i) => ({ ...row, trackNo: offset + i + 1 })),
      total: all.length,
      offset,
      nextOffset: next < all.length ? next : null,
      unavailable: null,
    });
  };

  const intakeCounts = (): Promise<IntakeCounts> =>
    source.wait({ intake: positions("intake").length, problems: problemNamed ? positions("problems").length : null });

  return { intakeQueue, intakeCounts };
}
