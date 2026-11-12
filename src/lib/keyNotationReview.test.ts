import rawSpellings from "./keySpellings.fixture.tsv?raw";
import { expect, it } from "vitest";
import { fromCamelot, toCamelot, transposeKey } from "./camelot";
import { keyLight } from "./keyLights";
import { DEFAULT_PREFERENCES, formatKey, sanitisePreferences } from "./preferences";
import { specForNode } from "./viewSpec";

const spellings = rawSpellings.trimEnd().split("\n").map(line => line.split("\t") as [string, string]);

it.each(spellings)("recognises file spelling %s without changing storage", (stored, code) => {
  expect(toCamelot(stored)).toBe(code);
  expect(formatKey(stored, "alphanumeric")).toBe(code);
  expect(formatKey(stored, "classic")).toBe(fromCamelot(code));
  expect(keyLight(code, stored)).toBe("match");
  expect(keyLight(stored, code)).toBe("match");
  expect(toCamelot(transposeKey(stored, 2))).toBe(toCamelot(transposeKey(code, 2)));
});

it.each(["Amazing", "Cminorish", "13m", "0d", "8AB", "Unknown", "H minor", ""])("leaves unknown %s visible and unlit", (stored) => {
  expect(toCamelot(stored)).toBe("");
  expect(formatKey(stored, "alphanumeric")).toBe(stored);
  expect(formatKey(stored, "classic")).toBe(stored);
  expect(keyLight("Am", stored)).toBe("none");
});

it("automatically follows display when sorting, including after a full save", () => {
  const fresh = sanitisePreferences(undefined);
  expect(fresh.view.keySort).toBe("auto");
  const full = { ...fresh, view: { ...fresh.view } };
  const sort = { column: "key" as const, descending: false };
  for (const display of ["alphanumeric", "classic"] as const) {
    const view = sanitisePreferences({ ...full, view: { ...full.view, keyDisplay: display } }).view;
    expect(specForNode(null, "", sort, view.keyDisplay, view.keySort).sort).toBe(display === "classic" ? "key" : "keyCamelot");
    for (const chosen of ["alphabetical", "musical"] as const) {
      const explicit = sanitisePreferences({ ...full, view: { ...view, keySort: chosen } }).view;
      expect(specForNode(null, "", sort, explicit.keyDisplay, explicit.keySort).sort).toBe(chosen === "musical" ? "keyCamelot" : "key");
    }
  }
});

it("preserves both existing display choices in a full saved preferences object", () => {
  for (const keyDisplay of ["classic", "alphanumeric"] as const) {
    const full = { ...DEFAULT_PREFERENCES, view: { ...DEFAULT_PREFERENCES.view } };
    full.view = { ...full.view, keyDisplay, keySort: "alphabetical", uiScale: 6 };
    expect(sanitisePreferences(full).view).toEqual(full.view);
  }
});

it("keeps stored notation independent when display preferences change", () => {
  const full = sanitisePreferences(undefined);
  expect(full.analysis.keyNotation).toBe("classic");
  full.view.keyDisplay = "classic";
  expect(sanitisePreferences(full).analysis.keyNotation).toBe("classic");
  full.analysis.keyNotation = "alphanumeric";
  expect(sanitisePreferences(full).analysis.keyNotation).toBe("alphanumeric");
});

it("preserves legacy stored notation when a full old preferences object is upgraded", () => {
  for (const keyDisplay of ["classic", "alphanumeric"] as const) {
    const old = { ...DEFAULT_PREFERENCES, view: { ...DEFAULT_PREFERENCES.view }, analysis: { ...DEFAULT_PREFERENCES.analysis } };
    delete old.analysis.keyNotation;
    old.view.keyDisplay = keyDisplay;
    const migrated = sanitisePreferences(old);
    expect(migrated.analysis.keyNotation).toBe(keyDisplay);
    migrated.view.keyDisplay = keyDisplay === "classic" ? "alphanumeric" : "classic";
    expect(sanitisePreferences(migrated).analysis.keyNotation).toBe(keyDisplay);
  }
});
