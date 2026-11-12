import { toCamelot } from "./camelot";

export type KeyLight = "none" | "match" | "boost";

/** The Library wheel: same, ±1, relative mode; +2 is the energy lift.
 * Wraps 12 → 1. Unknown keys are shown as stored, but cannot be matched.
 */
export function keyLight(loaded: string | null, row: string): KeyLight {
  const reference = toCamelot(loaded ?? "");
  const candidate = toCamelot(row);
  if (!reference || !candidate) return "none";
  const distance = (Number(candidate.slice(0, -1)) - Number(reference.slice(0, -1)) + 12) % 12;
  if (distance === 0) return "match";
  if (candidate.slice(-1) !== reference.slice(-1)) return "none";
  if (distance === 1 || distance === 11) return "match";
  if (distance === 2) return "boost";
  return "none";
}
