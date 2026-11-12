import type { RowDto } from "./types";

/**
 * A constructed Library for the Track Filter bar's browser tests (wbx-57),
 * on only with `?filterFixture=1`. The same formulas as
 * `src-tauri/tests/track_filter_agrees.rs`, so a test can say exactly which
 * tracks a filter must leave. Track `i` of 0 to 29, titled `Bar 00` to
 * `Bar 29`:
 *
 * - BPM 120.00 + 0.25 i; key `8A, 9A, 11B, 8A, Am` by i mod 5, except 28 is
 *   Open Key `1m` and 29 is `Mystery`;
 * - Energy (i / 2) mod 6 (0 is none); Prep colour i mod 9 (0 is none);
 * - Situation: Peak when i is even, Warm-up when i mod 5 is 0; Components:
 *   Synth when i mod 4 is 0, Vocal when i mod 7 is 0;
 * - genre "Deep House" when i mod 3 is 1.
 *
 * "Melodic Vox" holds tracks 3 to 20 and the intelligent playlist "Fresh
 * 128s" the Deep House tracks.
 */
export const FILTER_FIXTURE_SIZE = 30;

const KEYS = ["8A", "9A", "11B", "8A", "Am"];

function keyOf(i: number): string {
  if (i === 28) return "1m";
  if (i === 29) return "Mystery";
  return KEYS[i % KEYS.length] ?? "";
}

export function filterFixtureRows(base: RowDto[]): RowDto[] {
  return base.slice(0, FILTER_FIXTURE_SIZE).map((row, i) => ({
    ...row,
    title: `Bar ${String(i).padStart(2, "0")}`,
    artist: "Fixture",
    album: "",
    comment: "",
    label: "",
    genre: i % 3 === 1 ? "Deep House" : "",
    bpmX100: 12_000 + 25 * i,
    key: keyOf(i),
    rating: Math.floor(i / 2) % 6,
    analysed: 1,
  }));
}

export function filterFixtureColors(): Uint8Array {
  return Uint8Array.from({ length: FILTER_FIXTURE_SIZE }, (_, i) => i % 9);
}

/** The mock's MyTag ids a fixture track carries (see `myTagCategories`). */
export function filterFixtureTags(i: number): string[] {
  const tags: string[] = [];
  if (i % 2 === 0) tags.push("t-peak");
  if (i % 5 === 0) tags.push("t-warm");
  if (i % 4 === 0) tags.push("t-synth");
  if (i % 7 === 0) tags.push("t-vocal");
  return tags;
}

/** Playlist memberships the fixture fixes, by mock playlist id, as row positions. */
export function filterFixtureMembers(): Map<string, number[]> {
  return new Map([
    ["pl-0-0", Array.from({ length: 18 }, (_, k) => k + 3)],
    ["smart-0", Array.from({ length: FILTER_FIXTURE_SIZE }, (_, i) => i).filter((i) => i % 3 === 1)],
  ]);
}
