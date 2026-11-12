import type { RowDto } from "./types";

/** Small, named wheel fixture; enabled only by ?keyLights in the browser mock. */
export function keyLightRows(rows: RowDto[]): RowDto[] {
  const keys = ["Am", "Dm", "Em", "C", "Bm", "Gm", "G", "", "Unknown", "F#m", "Dbm", "Abm"];
  return rows.slice(0, keys.length).map((row, i) => ({
    ...row, title: `Key light ${String(i + 1).padStart(2, "0")}`, key: keys[i]!, analysed: 1,
  }));
}
