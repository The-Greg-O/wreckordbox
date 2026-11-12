import { describe, expect, it } from "vitest";
import { removalQuestion, type RemovalMutant } from "./collectionRemoval";

const mutant = (over: Partial<RemovalMutant>): RemovalMutant => ({
  id: 3,
  playlistId: "900001",
  playlistName: "Friday",
  position: 2,
  trackId: "10003",
  title: "Night Bus",
  description: "+1 st",
  ...over,
});

describe("removalQuestion", () => {
  it("asks only about the tracks when no Mutant is affected", () => {
    expect(removalQuestion(2, [])).toBe(
      "Remove 2 tracks from the collection? This can’t be undone. The files stay where they are.",
    );
  });

  it("names every Mutant that would go, with its playlist, place and edits", () => {
    const text = removalQuestion(1, [
      mutant({}),
      mutant({ id: 7, playlistName: "Saturday", position: null, description: "none" }),
    ]);
    expect(text).toBe(
      "Remove 1 track from the collection? This can’t be undone. The files stay where they are.\n\n" +
        "These 2 Mutants will be removed with them:\n" +
        "• Mutant 3: “Night Bus” (entry 2 of Friday), +1 st\n" +
        "• Mutant 7: “Night Bus” (in Saturday)",
    );
  });

  it("says one Mutant in the singular", () => {
    expect(removalQuestion(1, [mutant({})])).toContain("This Mutant will be removed with them:");
  });

  it("says when the Mutants could not be checked instead of hiding it", () => {
    expect(removalQuestion(1, { error: "the Sidecar is busy" })).toBe(
      "Remove 1 track from the collection? This can’t be undone. The files stay where they are.\n\n" +
        "The Mutants of these tracks could not be checked: the Sidecar is busy",
    );
  });
});
