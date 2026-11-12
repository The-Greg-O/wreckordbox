import { describe, expect, it } from "vitest";

import { counterStep } from "./intakeCounter";

// A page of 200 at offset 200 of a queue of 450: rows 201 to 400.
const page = { length: 200, offset: 200, nextOffset: 400 as number | null };

describe("counterStep", () => {
  it("with nothing chosen, next is the page's first track and there is no previous", () => {
    expect(counterStep({ ...page, index: -1 }, 1)).toEqual({ kind: "row", at: 0 });
    expect(counterStep({ ...page, index: -1 }, -1)).toBeNull();
  });

  it("moves within the page", () => {
    expect(counterStep({ ...page, index: 5 }, 1)).toEqual({ kind: "row", at: 6 });
    expect(counterStep({ ...page, index: 5 }, -1)).toEqual({ kind: "row", at: 4 });
  });

  it("from the page's last track, next reads the next page's first", () => {
    expect(counterStep({ ...page, index: 199 }, 1)).toEqual({ kind: "page", read: 400, offset: 400 });
  });

  it("from the page's first track, previous reads the previous page's last", () => {
    expect(counterStep({ ...page, index: 0 }, -1)).toEqual({ kind: "page", read: 199, offset: 0 });
    // A page that starts off the 200s (after a mark moved on) never goes below 0.
    expect(counterStep({ ...page, offset: 57, index: 0 }, -1)).toEqual({ kind: "page", read: 56, offset: 0 });
  });

  it("stops at the queue's ends", () => {
    expect(counterStep({ length: 3, offset: 0, nextOffset: null, index: 2 }, 1)).toBeNull();
    expect(counterStep({ length: 3, offset: 0, nextOffset: null, index: 0 }, -1)).toBeNull();
    expect(counterStep({ length: 0, offset: 0, nextOffset: null, index: -1 }, 1)).toBeNull();
  });
});
