import { describe, expect, it } from "vitest";
import { rowKeyVerb, sinceFocus, type RowAtFocus } from "./reviewRowKeys";

const at: RowAtFocus = { fact: 7, token: "r7.0", index: 2, top: 64, text: "Track A Energy 3 1 0.91", marked: null };

describe("sinceFocus", () => {
  it("is null for the same row in the same place showing the same thing", () => {
    expect(sinceFocus(at, { ...at })).toBeNull();
  });
  it("says changed when the row's token or what it shows differs", () => {
    expect(sinceFocus(at, { ...at, token: "r7.1" })).toBe("changed");
    expect(sinceFocus(at, { ...at, text: "Track A Energy 3 5 0.91" })).toBe("changed");
  });
  it("says moved when the row is at another position or height in the list", () => {
    expect(sinceFocus(at, { ...at, index: 1 })).toBe("moved");
    expect(sinceFocus(at, { ...at, top: 32 })).toBe("moved");
  });
  it("says arrived when the row is newly marked as arrived, or is another row", () => {
    expect(sinceFocus(at, { ...at, marked: "added" })).toBe("arrived");
    expect(sinceFocus(at, { ...at, fact: 8 })).toBe("arrived");
  });
});

describe("rowKeyVerb", () => {
  const key = (k: string, mods: Partial<{ ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean }> = {}) =>
    ({ key: k, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...mods });
  it("is accept for Y and reject for N, in either case", () => {
    expect(rowKeyVerb(key("y"))).toBe("accept");
    expect(rowKeyVerb(key("Y"))).toBe("accept");
    expect(rowKeyVerb(key("n"))).toBe("reject");
    expect(rowKeyVerb(key("N", { shiftKey: true }))).toBe("reject");
  });
  it("is nothing with Ctrl, Cmd or Alt (Ctrl+Y is Redo) or for another key", () => {
    expect(rowKeyVerb(key("y", { ctrlKey: true }))).toBeNull();
    expect(rowKeyVerb(key("y", { metaKey: true }))).toBeNull();
    expect(rowKeyVerb(key("n", { altKey: true }))).toBeNull();
    expect(rowKeyVerb(key("m"))).toBeNull();
  });
});
