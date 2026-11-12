import { describe, expect, it } from "vitest";

import { keyColumnWidth } from "./keyColumnWidth";

/** Every character 10 px wide, except the ones named. */
const measure = (wide: Record<string, number> = {}) => (text: string, bold: boolean): number =>
  [...text].reduce((sum, ch) => sum + (wide[ch] ?? 10) * (bold ? 1.5 : 1), 0);

describe("the Key column's width", () => {
  it("holds the widest key with its badge, bold mark and the cell's padding", () => {
    // "10A" 30 + gap 2 + bold mark 15 + badge padding 4 = 51; cell padding and rule 13.
    expect(keyColumnWidth(measure())).toBe(64);
  });

  it("follows Classic spellings when they are the wider ones", () => {
    // "F#m" 10 + 40 + 10 = 60, + 2 + 15 + 4 + 13.
    expect(keyColumnWidth(measure({ "#": 40 }))).toBe(94);
  });

  it("follows the wider of the two marks", () => {
    // "10A" 30 + 2 + bold "↑" 30 + 4 + 13.
    expect(keyColumnWidth(measure({ "↑": 20 }))).toBe(79);
  });

  it("rounds a fractional measure up, never down", () => {
    // 18.03 + 2 + 6.01 + 4 = 30.04, so 31; + 13.
    expect(keyColumnWidth((text) => text.length * 6.01)).toBe(44);
  });
});
