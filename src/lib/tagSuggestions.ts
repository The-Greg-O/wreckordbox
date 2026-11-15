/**
 * Which proposed tag belongs on a chip (wbx-45.26).
 *
 * Intake lists an agent's proposed tag as a Review row: `field` is the
 * dimension, `terms` are the term ids, and accepting any of those chips
 * accepts that whole row. The service lists the newest proposal first, so
 * the first suggestion that names a term is the one the chip shows.
 */
import type { IntakeSuggestion } from "./intakeQueue";

/** The proposal for `term` of `dimension`, or `null` when none is waiting. */
export function proposalFor(
  suggestions: readonly IntakeSuggestion[],
  dimension: string,
  term: string,
): IntakeSuggestion | null {
  for (const suggestion of suggestions) {
    if (suggestion.field !== dimension) continue;
    if (suggestion.terms?.includes(term)) return suggestion;
  }
  return null;
}
