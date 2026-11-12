/** A key prefix as found in file tags. The suffix is annotation, not storage.
 * Require a token boundary so e.g. "Amazing" and "Cminorish" are not keys.
 * Open Key 1m/1d correspond to Camelot 8A/8B.
 */
export function keySpelling(raw: string): { pitch: number; minor: boolean } | { code: string } | null {
  const text = raw.trim().replaceAll("♯", "#").replaceAll("♭", "b");
  const code = /^(\d{1,2})([abmd])(?=$|[\s/,:;()[\]–—-])/i.exec(text);
  if (code) {
    const number = Number(code[1]);
    if (number < 1 || number > 12) return null;
    const mode = code[2]!.toLowerCase();
    return { code: mode === "a" || mode === "b"
      ? `${number}${mode.toUpperCase()}`
      : `${(number + 6) % 12 + 1}${mode === "m" ? "A" : "B"}` };
  }
  const note = /^([a-g])([#b]?)(?:\s*(minor|major|min|maj|m))?(?=$|[\s/,:;()[\]–—-])/i.exec(text);
  if (!note) return null;
  const natural: Record<string, number> = { c: 0, d: 2, e: 4, f: 5, g: 7, a: 9, b: 11 };
  const pitch = natural[note[1]!.toLowerCase()]! + (note[2] === "#" ? 1 : note[2]?.toLowerCase() === "b" ? -1 : 0);
  const suffix = note[3]?.toLowerCase();
  return { pitch: (pitch + 12) % 12, minor: suffix === "minor" || suffix === "min" || suffix === "m" };
}
