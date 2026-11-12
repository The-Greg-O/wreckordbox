import { describe, expect, it } from "vitest";

import type { FocusTarget } from "./shortcuts";
import { fromServiceColours, INTAKE_ACTIONS, intakeActionForKey, nextQueuedIndex } from "./intakeEdits";

const linux = { mac: false };
const body: FocusTarget = { tagName: "BODY", closest: () => null };
const field: FocusTarget = { tagName: "INPUT", closest: () => null };
const inDialog: FocusTarget = { tagName: "BUTTON", closest: () => ({}) };

describe("intakeActionForKey", () => {
  it("Ctrl+Enter marks Clean and next, P marks Problem, ] skips", () => {
    expect(intakeActionForKey({ key: "Enter", ctrlKey: true }, linux, body)).toBe("intakeMarkClean");
    expect(intakeActionForKey({ key: "p", code: "KeyP" }, linux, body)).toBe("intakeMarkProblem");
    expect(intakeActionForKey({ key: "P", code: "KeyP", shiftKey: true }, linux, body)).toBeNull();
    expect(intakeActionForKey({ key: "]", code: "BracketRight" }, linux, body)).toBe("intakeSkip");
  });

  it("Shift with 1 to 5 sets Energy, however the layout spells the shifted digit", () => {
    expect(intakeActionForKey({ key: "!", code: "Digit1", shiftKey: true }, linux, body)).toBe("intakeEnergy1");
    expect(intakeActionForKey({ key: "%", code: "Digit5", shiftKey: true }, linux, body)).toBe("intakeEnergy5");
    // Plain digits are the views' (1 to 4); 6 is nothing.
    expect(intakeActionForKey({ key: "3", code: "Digit3" }, linux, body)).toBeNull();
    expect(intakeActionForKey({ key: "^", code: "Digit6", shiftKey: true }, linux, body)).toBeNull();
  });

  it("leaves keys to a text field and to a dialog", () => {
    expect(intakeActionForKey({ key: "p", code: "KeyP" }, linux, field)).toBeNull();
    expect(intakeActionForKey({ key: "Enter", ctrlKey: true }, linux, field)).toBeNull();
    expect(intakeActionForKey({ key: "]", code: "BracketRight" }, linux, inDialog)).toBeNull();
  });

  it("follows the DJ's own key, and an emptied one unbinds", () => {
    expect(intakeActionForKey({ key: "s", code: "KeyS" }, linux, body, { intakeSkip: { key: "s" } })).toBe("intakeSkip");
    expect(intakeActionForKey({ key: "]", code: "BracketRight" }, linux, body, { intakeSkip: { key: "s" } })).toBeNull();
    expect(intakeActionForKey({ key: "p", code: "KeyP" }, linux, body, { intakeMarkProblem: { key: "" } })).toBeNull();
  });

  it("names every action once", () => {
    const actions = INTAKE_ACTIONS.map((entry) => entry.action);
    expect(new Set(actions).size).toBe(actions.length);
    expect(actions).toEqual([
      "intakeMarkClean", "intakeMarkProblem", "intakeSkip",
      "intakeEnergy1", "intakeEnergy2", "intakeEnergy3", "intakeEnergy4", "intakeEnergy5",
      "intakeTag",
    ]);
  });
});

describe("fromServiceColours", () => {
  it("keeps the eight labels in order and which mean Clean and Problem", () => {
    const green = { id: 5, colour: "Green", label: "Clean" };
    const red = { id: 2, colour: "Red", label: "Problem" };
    const colours = fromServiceColours({
      colours: [{ id: 1, colour: "Pink", label: "Buy" }, red, green],
      clean: green,
      problem: red,
      problem_unavailable: undefined,
    });
    expect(colours.labels.map((label) => label.label)).toEqual(["Buy", "Problem", "Clean"]);
    expect(colours.clean?.id).toBe(5);
    expect(colours.problem?.id).toBe(2);
  });

  it("has no Clean or Problem when the Library names none", () => {
    const colours = fromServiceColours({ colours: [], clean: null, problem: null });
    expect(colours.clean).toBeNull();
    expect(colours.problem).toBeNull();
  });
});

describe("nextQueuedIndex", () => {
  it("is the row after the current one on the page", () => {
    expect(nextQueuedIndex(4, 1)).toBe(2);
  });
  it("is the first row when nothing is chosen", () => {
    expect(nextQueuedIndex(4, -1)).toBe(0);
  });
  it("is nothing past the page's end", () => {
    expect(nextQueuedIndex(4, 3)).toBeNull();
    expect(nextQueuedIndex(0, -1)).toBeNull();
  });
});
