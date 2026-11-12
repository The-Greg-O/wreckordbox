import { describe, expect, it } from "vitest";

import { dispatch, type Platform } from "./shortcuts";
import { SHELL_VIEWS, shellViewForKey } from "./shellViews";

const linux: Platform = { mac: false, linux: true };
const list = { tagName: "DIV" };

describe("the shell's views", () => {
  it("are Library, Intake, Review and Stick, in that order", () => {
    expect(SHELL_VIEWS.map((view) => view.id)).toEqual(["library", "intake", "review", "stick"]);
    expect(SHELL_VIEWS.map((view) => view.label)).toEqual(["Library", "Intake", "Review", "Stick"]);
  });

  it("are on keys 1 to 4", () => {
    expect(shellViewForKey({ key: "1" }, linux, list)).toBe("library");
    expect(shellViewForKey({ key: "2" }, linux, list)).toBe("intake");
    expect(shellViewForKey({ key: "3" }, linux, list)).toBe("review");
    expect(shellViewForKey({ key: "4" }, linux, list)).toBe("stick");
    expect(shellViewForKey({ key: "5" }, linux, list)).toBeNull();
    expect(shellViewForKey({ key: "a" }, linux, list)).toBeNull();
  });

  it("leave the key alone while the DJ is typing in a field", () => {
    expect(shellViewForKey({ key: "2" }, linux, { tagName: "INPUT" })).toBeNull();
    expect(shellViewForKey({ key: "2" }, linux, { tagName: "TEXTAREA" })).toBeNull();
    expect(shellViewForKey({ key: "2" }, linux, { isContentEditable: true })).toBeNull();
  });

  it("leave the key alone inside a dialog or a confirmation prompt", () => {
    for (const within of ['[role="dialog"]', '[role="alertdialog"]', "dialog"]) {
      const target = { tagName: "BUTTON", closest: (selector: string) => (selector.split(", ").includes(within) ? {} : null) };
      expect(shellViewForKey({ key: "2" }, linux, target), within).toBeNull();
    }
  });

  it("are the bare key only: a modified digit is some other command's", () => {
    // shift + 1 is Player B's Set Hot Cue A and ctrl + 1 Clear Hot Cue A.
    expect(shellViewForKey({ key: "!", code: "Digit1", shiftKey: true }, linux, list)).toBeNull();
    expect(shellViewForKey({ key: "1", ctrlKey: true }, linux, list)).toBeNull();
    expect(shellViewForKey({ key: "1", altKey: true }, linux, list)).toBeNull();
  });

  it("follow a key the DJ gave a view, or took away", () => {
    const overrides = { viewIntake: { key: "F5" }, viewStick: { key: "" } };
    expect(shellViewForKey({ key: "F5" }, linux, list, overrides)).toBe("intake");
    expect(shellViewForKey({ key: "2" }, linux, list, overrides)).toBeNull();
    expect(shellViewForKey({ key: "4" }, linux, list, overrides)).toBeNull();
    expect(shellViewForKey({ key: "1" }, linux, list, overrides)).toBe("library");
  });

  it("do not take 1 to 4 from the deck in the key map the old layout reads", () => {
    expect(dispatch({ key: "1" }, linux, list)).toBe("hotCueA");
    expect(dispatch({ key: "2" }, linux, list)).toBe("hotCueB");
    expect(dispatch({ key: "3" }, linux, list)).toBe("hotCueC");
    expect(dispatch({ key: "4" }, linux, list)).toBe("beatLoop1");
  });
});
