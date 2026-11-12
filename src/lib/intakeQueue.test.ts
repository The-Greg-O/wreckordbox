import { describe, expect, it } from "vitest";

import type { RowDto } from "@/ipc/types";
import { fromServicePage, needsLine, queuePosition, type QueuedTrack } from "./intakeQueue";

const track = (over: Partial<QueuedTrack> = {}): QueuedTrack => ({
  id: "1", title: "T", artist: "A", bpm: 124, key: "8A",
  needs: [], emptyDimensions: [], grid: "confirmed", suggestions: [],
  ...over,
});

describe("needsLine", () => {
  it("names what is missing, in the service's order, in the DJ's words", () => {
    expect(needsLine(track({ needs: ["title", "energy", "tags", "hot_cues"], emptyDimensions: [{ id: "1", name: "Mood" }, { id: "2", name: "Situation" }] })))
      .toBe("needs title · Energy · tags · cues");
  });

  it("names the one empty dimension rather than saying tags", () => {
    expect(needsLine(track({ needs: ["tags", "grid"], emptyDimensions: [{ id: "3", name: "Situation" }] })))
      .toBe("needs Situation · grid");
  });

  it("says everything when Energy, tags, cues and grid are all missing", () => {
    expect(needsLine(track({ needs: ["energy", "tags", "hot_cues", "grid"] }))).toBe("needs everything");
    expect(needsLine(track({ needs: ["title", "energy", "tags", "hot_cues", "grid"] }))).toBe("needs everything");
  });

  it("says nothing is missing when nothing is", () => {
    expect(needsLine(track())).toBe("nothing missing");
  });

  it("passes over a need it does not know, so a new one cannot break the list", () => {
    expect(needsLine(track({ needs: ["energy", "stems"] }))).toBe("needs Energy");
  });
});

describe("queuePosition", () => {
  it("reads as the design's 2 of 23", () => {
    expect(queuePosition(1, 23)).toBe("2 of 23");
    expect(queuePosition(-1, 23)).toBe("23");
    expect(queuePosition(0, 0)).toBe("0");
  });
});

describe("fromServicePage", () => {
  const row = { id: "10", title: "Row title" } as RowDto;

  it("lifts the service's queue into the view's terms, with each track's row", () => {
    const page = fromServicePage({
      queue: {
        tracks: [{
          id: "10", title: "Row title", artist: "Who", bpm: 125.5, key: "7A", energy: null,
          needs: ["energy", "tags"], empty_dimensions: [{ id: "3", name: "Situation", kind: "mytag_category" }],
          grid: "not_confirmed",
          suggestions: [{ field: "title", fact_id: 4, value: "Better", source: { kind: "agent", name: "tagger" }, confidence: 0.9 }],
        }],
        total: 61, offset: 0, limit: 50, next_offset: 50, prep_colour: null, suggestions_unavailable: "the Sidecar is busy",
      },
      rows: [row],
      unavailable: null,
    });

    expect(page.total).toBe(61);
    expect(page.nextOffset).toBe(50);
    expect(page.rows).toEqual([row]);
    expect(page.unavailable).toBeNull();
    expect(page.suggestionsUnavailable).toBe("the Sidecar is busy");
    expect(page.tracks[0]).toEqual({
      id: "10", title: "Row title", artist: "Who", bpm: 125.5, key: "7A",
      needs: ["energy", "tags"], emptyDimensions: [{ id: "3", name: "Situation" }], grid: "not_confirmed",
      suggestions: [{ field: "title", value: "Better", source: "tagger", confidence: 0.9 }],
    });
  });

  it("keeps why a queue cannot be listed", () => {
    const page = fromServicePage({ queue: null, rows: [], unavailable: { code: "problem_colour_not_found", message: "No colour is named Problem." } });
    expect(page.tracks).toEqual([]);
    expect(page.total).toBe(0);
    expect(page.unavailable).toBe("No colour is named Problem.");
  });
});
