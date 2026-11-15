import { describe, expect, it } from "vitest";

import { acceptKey, proposalSince } from "./IntakeSuggestions";

const plain = { ctrlKey: false, metaKey: false, altKey: false };

describe("Y on a proposal (wbx-66, Review's rules from wbx-mvr)", () => {
  it("is a plain Y, Enter or Space: never with Ctrl (Redo), Cmd or Alt", () => {
    expect(acceptKey({ key: "y", ...plain })).toBe("Y");
    expect(acceptKey({ key: "Y", ...plain })).toBe("Y");
    expect(acceptKey({ key: "Enter", ...plain })).toBe("Enter");
    expect(acceptKey({ key: " ", ...plain })).toBe("Space");
    expect(acceptKey({ key: "y", ...plain, ctrlKey: true })).toBeNull();
    expect(acceptKey({ key: "Enter", ...plain, metaKey: true })).toBeNull();
    expect(acceptKey({ key: "y", ...plain, altKey: true })).toBeNull();
    expect(acceptKey({ key: "n", ...plain })).toBeNull();
  });

  it("accepts only the proposal as it was when it took focus", () => {
    const at = { fact: 7, token: "t1", value: "A Title" };
    expect(proposalSince(at, { ...at })).toBeNull();
    expect(proposalSince(at, null)).toBe("gone");
    expect(proposalSince(at, { ...at, fact: 8 })).toBe("gone");
    expect(proposalSince(at, { ...at, token: "t2" })).toBe("changed");
    expect(proposalSince(at, { ...at, value: "Another" })).toBe("changed");
  });
});
