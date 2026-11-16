import { describe, expect, it } from "vitest";
import type { IntakeSuggestion } from "./intakeQueue";
import { proposalFor } from "./tagSuggestions";

function suggestion(over: Partial<IntakeSuggestion> & Pick<IntakeSuggestion, "field" | "fact">): IntakeSuggestion {
  return {
    value: "Peak",
    source: "tagger",
    confidence: 0.84,
    token: `t:${over.fact}`,
    before: "",
    ...over,
  };
}

describe("proposalFor (wbx-45.26)", () => {
  const peak = suggestion({ field: "setting", fact: 7, terms: ["peak"], value: "Peak" });
  const both = suggestion({ field: "setting", fact: 8, terms: ["peak", "warm-up"], value: "Peak" });
  const older = suggestion({ field: "setting", fact: 3, terms: ["peak"], value: "Peak", token: "old" });
  const other = suggestion({ field: "feel", fact: 9, terms: ["dark"], value: "Dark" });
  const title = suggestion({ field: "title", fact: 1, value: "A Better Title" });

  it("names the suggestion whose field is the dimension and whose terms include the chip", () => {
    expect(proposalFor([title, peak], "setting", "peak")).toEqual(peak);
    expect(proposalFor([title, peak], "setting", "warm-up")).toBeNull();
    expect(proposalFor([other], "setting", "peak")).toBeNull();
  });

  it("marks every term of one suggestion with that same suggestion", () => {
    expect(proposalFor([both], "setting", "peak")).toBe(both);
    expect(proposalFor([both], "setting", "warm-up")).toBe(both);
  });

  it("keeps the first match: the service lists the newest proposal first", () => {
    expect(proposalFor([peak, older], "setting", "peak")?.fact).toBe(7);
    expect(proposalFor([older, peak], "setting", "peak")?.fact).toBe(3);
  });
});
