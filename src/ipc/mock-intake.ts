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
 *
 * Hooks on `window` (wbx-62 review): `__intakeReads()` counts the reads
 * asked of each command; `__holdIntakeReads()` holds every read from then
 * on until `__releaseIntakeReads()`; `__failIntakeReads(on)` makes reads
 * fail. Until the mock's Library is ready (`?slow`, then `__libraryReady()`)
 * both commands fail as the real ones do while the Library loads.
 */
import type { RowDto } from "./types";
import type { IntakeColours, PrepColour } from "@/lib/intakeEdits";
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
  /** Whether the mock's Library has finished loading. */
  ready: () => boolean;
  /** The mock's own undoable colour edit (`setTrackColor`), as the service's `tracks.set_prep_colour` makes it (wbx-63). */
  setColour: (id: string, colour: number | null) => Promise<unknown>;
}

/**
 * The mock's colour labels as a DJ names them (wbx-63): Clean is Green
 * and Problem is Red, as above; Pink, Orange and Blue are renamed too, the
 * rest keep their colour's name. `?noProblemColour` leaves Red as "Red".
 */
const COLOUR_NAMES = ["Pink", "Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple"] as const;
const DJ_LABELS: Partial<Record<number, string>> = { 1: "Sample", 2: "Problem", 3: "Buy", 5: "Clean", 7: "Delete" };

function readUrl(): URLSearchParams {
  return typeof location === "undefined" ? new URLSearchParams() : new URLSearchParams(location.search);
}

export function createMockIntake(source: MockIntakeSource) {
  const reads = { counts: 0, queue: 0 };
  let holding = false;
  let failing = false;
  let held: (() => void)[] = [];
  if (typeof window !== "undefined") {
    const hooks = window as unknown as {
      __intakeReads: () => { counts: number; queue: number };
      __holdIntakeReads: () => void;
      __releaseIntakeReads: () => void;
      __failIntakeReads: (on: boolean) => void;
    };
    hooks.__intakeReads = () => ({ ...reads });
    hooks.__holdIntakeReads = () => { holding = true; };
    hooks.__releaseIntakeReads = () => {
      holding = false;
      const waiting = held;
      held = [];
      for (const release of waiting) release();
    };
    hooks.__failIntakeReads = (on) => { failing = on; };
  }
  /** Every read: counted, held while holding, refused while loading or failing. Answered as it stands when it runs. */
  const guarded = async <T>(which: keyof typeof reads, answer: () => Promise<T>): Promise<T> => {
    reads[which] += 1;
    if (holding) await new Promise<void>((resolve) => { held.push(resolve); });
    if (!source.ready()) throw new Error("The library has not finished loading yet.");
    if (failing) throw new Error("The Library service did not answer.");
    return answer();
  };
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

  const intakeQueue = (queue: IntakeQueueName, offset: number, limit: number): Promise<IntakeQueuePage> =>
    guarded("queue", () => readQueue(queue, offset, limit));
  const readQueue = (queue: IntakeQueueName, offset: number, limit: number): Promise<IntakeQueuePage> => {
    if (queue === "problems" && !problemNamed) {
      return source.wait({ tracks: [], rows: [], total: 0, offset, nextOffset: null, unavailable: NO_PROBLEM_COLOUR, suggestionsUnavailable: null });
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
      suggestionsUnavailable: null,
    });
  };

  const labels: PrepColour[] = COLOUR_NAMES.map((colour, at) => {
    const id = at + 1;
    const named = id === MOCK_PROBLEM_COLOUR && !problemNamed ? undefined : DJ_LABELS[id];
    return { id, colour, label: named ?? colour };
  });
  const intakeColours = (): Promise<IntakeColours> => source.wait({
    labels: labels.map((label) => ({ ...label })),
    clean: { ...labels[MOCK_CLEAN_COLOUR - 1]! },
    problem: problemNamed ? { ...labels[MOCK_PROBLEM_COLOUR - 1]! } : null,
  });
  /** Every Prep colour edit asked of the "service", in order: what a test checks really landed. */
  const prepWrites: { id: string; colour: number | null }[] = [];
  const intakeSetPrepColour = async (id: string, colour: number | null): Promise<void> => {
    prepWrites.push({ id, colour });
    if (colour !== null && (colour < 1 || colour > 8)) throw new Error(`There is no colour label ${colour}.`);
    if (!source.all.some((row) => row.id === id)) throw new Error(`no track ${id}`);
    await source.setColour(id, colour);
  };
  if (typeof window !== "undefined") {
    const hooks = window as unknown as {
      __prepWrites: () => { id: string; colour: number | null }[];
      __prepColourOf: (id: string) => number;
    };
    hooks.__prepWrites = () => prepWrites.map((write) => ({ ...write }));
    hooks.__prepColourOf = (id) => {
      const at = source.all.findIndex((row) => row.id === id);
      return at < 0 ? -1 : (source.colors[at] ?? 0);
    };
  }

  const intakeCounts = (): Promise<IntakeCounts> => guarded("counts", () =>
    source.wait({ intake: positions("intake").length, problems: problemNamed ? positions("problems").length : null }));

  return { intakeQueue, intakeCounts, intakeColours, intakeSetPrepColour };
}
