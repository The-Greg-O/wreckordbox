import type { RowDto, TagCategory, TagPick } from "./types";

/**
 * The mock Stick's own MyTags (wbx-45.12), in both of its libraries, as the
 * Rust side reads them from `exportExt.pdb` / `exportLibrary.db`: two
 * categories in the Stick's order, "Vibe" (Dark, Uplifting) and "Situation"
 * (Peak, Warm-up), which differ from the Library's. By the mock Stick's
 * track titles (made-up tracks, `mock-device-library.ts`):
 *
 * - B2ME: Uplifting, Peak
 * - Spectrum: Dark, Uplifting, Peak
 * - Lost Without You: Dark, Warm-up
 * - Gravity: none
 * - Mirage: Warm-up
 */
export const STICK_MY_TAGS: readonly TagCategory[] = [
  { name: "Vibe", tags: ["Dark", "Uplifting"] },
  { name: "Situation", tags: ["Peak", "Warm-up"] },
];

/** Each track's tags as `"Category: Tag"`. */
const CARRIED: ReadonlyMap<string, readonly string[]> = new Map([
  ["B2ME (feat. JordinLaine) (Original Mix)", ["Vibe: Uplifting", "Situation: Peak"]],
  ["Spectrum (Extended Mix)", ["Vibe: Dark", "Vibe: Uplifting", "Situation: Peak"]],
  ["Lost Without You", ["Vibe: Dark", "Situation: Warm-up"]],
  ["Mirage", ["Situation: Warm-up"]],
]);

/**
 * Whether a Stick row passes the bar's tag picks, by the Rust side's rule:
 * a pick's category is its place among the Stick's; All or Any within it;
 * every column together; a name the category does not hold, or a category
 * past the Stick's, admits nothing; a column with nothing picked, anything.
 */
export function passesStickTags(row: RowDto, picks: readonly TagPick[] | undefined): boolean {
  const carried = CARRIED.get(row.title) ?? [];
  return (picks ?? []).every((pick) => {
    if (pick.tags.length === 0) return true;
    const category = STICK_MY_TAGS[pick.category];
    const has = (tag: string) => category !== undefined && category.tags.includes(tag) && carried.includes(`${category.name}: ${tag}`);
    return pick.any ? pick.tags.some(has) : pick.tags.every(has);
  });
}
