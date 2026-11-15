import { describe, expect, it } from "vitest";

import { acceptsKey, proposalSince } from "./IntakeSuggestions";

const plain = { ctrlKey: false, metaKey: false, altKey: false };

describe("Y on a proposal (wbx-66, Review's rules from wbx-mvr)", () => {
  it("is a plain Y only: never with Ctrl (Redo), Cmd or Alt", () => {
    expect(acceptsKey({ key: "y", ...plain })).toBe(true);
    expect(acceptsKey({ key: "Y", ...plain })).toBe(true);
    expect(acceptsKey({ key: "y", ...plain, ctrlKey: true })).toBe(false);
    expect(acceptsKey({ key: "y", ...plain, metaKey: true })).toBe(false);
    expect(acceptsKey({ key: "y", ...plain, altKey: true })).toBe(false);
    expect(acceptsKey({ key: "n", ...plain })).toBe(false);
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
