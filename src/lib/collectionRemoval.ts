/**
 * What Remove from Collection asks before it removes anything (wbx-80):
 * the tracks, and every Mutant that would go with them. A Mutant is a
 * playlist's own version of a track; removing the track removes it, so the
 * DJ is told which before confirming. When the Mutants could not be read
 * the question says so rather than staying silent.
 */

/** One Mutant that would go with the tracks. */
export interface RemovalMutant {
  id: number;
  playlistId: string;
  playlistName: string;
  /** Its place in the playlist, from 1; null when the entry is gone. */
  position: number | null;
  trackId: string;
  title: string;
  /** What differs from the parent, in words ("+1 st"). */
  description: string;
}

/** The Mutants, or why they could not be read. */
export type RemovalMutants = readonly RemovalMutant[] | { error: string };

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** The confirmation's text for removing `count` tracks. */
export function removalQuestion(count: number, mutants: RemovalMutants): string {
  const asked = `Remove ${plural(count, "track")} from the collection? This can’t be undone. The files stay where they are.`;
  if ("error" in mutants) {
    return `${asked}\n\nThe Mutants of these tracks could not be checked: ${mutants.error}`;
  }
  if (mutants.length === 0) return asked;
  const lines = mutants.map((m) => {
    const place = m.position === null ? `in ${m.playlistName}` : `entry ${m.position} of ${m.playlistName}`;
    const what = m.description && m.description !== "none" ? `, ${m.description}` : "";
    return `• Mutant ${m.id}: “${m.title}” (${place})${what}`;
  });
  const heading =
    mutants.length === 1
      ? "This Mutant will be removed with them:"
      : `These ${mutants.length} Mutants will be removed with them:`;
  return `${asked}\n\n${heading}\n${lines.join("\n")}`;
}
