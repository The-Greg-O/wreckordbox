import { describe, expect, it } from "vitest";
import { fromCamelot, toCamelot } from "./camelot";
import { keyLight } from "./keyLights";
import { DEFAULT_PREFERENCES, formatKey, sanitisePreferences } from "./preferences";

const KEYS = [
  ["Abm", "B"], ["Ebm", "F#"], ["Bbm", "Db"], ["Fm", "Ab"],
  ["Cm", "Eb"], ["Gm", "Bb"], ["Dm", "F"], ["Am", "C"],
  ["Em", "G"], ["Bm", "D"], ["F#m", "A"], ["Dbm", "E"],
] as const;

describe("Library key notation", () => {
  it("converts all 24 keys in both directions and displays either stored notation", () => {
    KEYS.forEach((pair, i) => pair.forEach((classic, mode) => {
      const code = `${i + 1}${mode === 0 ? "A" : "B"}`;
      expect(toCamelot(classic)).toBe(code);
      expect(fromCamelot(code)).toBe(classic);
      for (const stored of [classic, code]) {
        expect(formatKey(stored, "alphanumeric")).toBe(code);
        expect(formatKey(stored, "classic")).toBe(classic);
      }
    }));
  });

  it("defaults to Camelot and preserves an explicitly saved Classic choice", () => {
    expect(DEFAULT_PREFERENCES.view.keyDisplay).toBe("alphanumeric");
    expect(sanitisePreferences({ view: {} }).view.keyDisplay).toBe("alphanumeric");
    expect(sanitisePreferences({ view: { keyDisplay: "invalid" } }).view.keyDisplay).toBe("alphanumeric");
    expect(sanitisePreferences({ view: { keyDisplay: "classic" } }).view.keyDisplay).toBe("classic");
  });
});

describe("Library key lights", () => {
  it("matches the same key, either neighbour and relative mode for every wheel position", () => {
    for (let n = 1; n <= 12; n++) {
      for (const mode of ["A", "B"]) {
        const loaded = `${n}${mode}`;
        for (let row = 1; row <= 12; row++) {
          for (const rowMode of ["A", "B"]) {
            const distance = (row - n + 12) % 12;
            const expected = distance === 0 || (mode === rowMode && [1, 11].includes(distance))
              ? "match" : mode === rowMode && distance === 2 ? "boost" : "none";
            expect(keyLight(loaded, `${row}${rowMode}`), `${loaded} → ${row}${rowMode}`).toBe(expected);
          }
        }
      }
    }
  });

  it("compares stored classic names, enharmonics and Camelot codes alike", () => {
    expect(keyLight("G#m", "Abm")).toBe("match");
    expect(keyLight("Am", "9a")).toBe("match");
    expect(keyLight("8A", "C")).toBe("match");
    expect(keyLight("Am", "Bm")).toBe("boost");
    expect(keyLight("11B", "B")).toBe("boost");
    expect(keyLight("12A", "Ebm")).toBe("boost");
    // The design's energy lift is +2, not the -2 drop or a mode switch.
    expect(keyLight("Am", "Gm")).toBe("none");
    expect(keyLight("Am", "D")).toBe("none");
  });

  it("never lights missing or unrecognised keys, including an empty deck", () => {
    for (const loaded of [null, "", "Unknown", "13A"]) expect(keyLight(loaded, "Am")).toBe("none");
    for (const row of ["", "Unknown", "0B"]) expect(keyLight("Am", row)).toBe("none");
  });
});
