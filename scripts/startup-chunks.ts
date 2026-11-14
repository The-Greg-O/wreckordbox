/**
 * Which built JavaScript chunks the main window loads before its first paint,
 * read from a production build's own files (no manifest is shipped).
 *
 * Start-up code is the module entry named in `index.html`, everything it
 * imports statically, and each chunk `src/main.tsx` awaits before its first
 * render (the main window's `App`), with everything those import statically.
 * Chunks reached only through a dynamic `import()` load later and do not count.
 * The obfuscator keeps import paths literal (`ignoreImports` in vite.config.ts),
 * so the specifiers can be read from the chunk text.
 */

const STATIC_IMPORT = /\bimport\s*(?:[\w$]+\s*,?\s*)?(?:\{[^}]*\}|\*\s*as\s*[\w$]+)?\s*(?:from\s*)?["']\.\/([\w.-]+\.js)["']/g;

function staticImports(code: string): string[] {
  return [...code.matchAll(STATIC_IMPORT)].map((m) => m[1]!);
}

/**
 * @param indexHtml the built `index.html`
 * @param chunks built chunk file name (as in `assets/`) to its code
 * @param awaitedBeforeFirstRender chunk names (`App` for `App-<hash>.js`) that
 *   the entry loads dynamically and awaits before it first renders
 * @returns the file names of every start-up chunk
 */
export function startupChunks(
  indexHtml: string,
  chunks: Record<string, string>,
  awaitedBeforeFirstRender: string[],
): string[] {
  const entry = /<script\s+type="module"[^>]*\ssrc="\/assets\/([\w.-]+\.js)"/.exec(indexHtml)?.[1];
  if (!entry || !(entry in chunks)) throw new Error("index.html names no module entry among the built chunks");
  const roots = [entry];
  for (const name of awaitedBeforeFirstRender) {
    const matches = Object.keys(chunks).filter((f) => new RegExp(`^${name}-[\\w-]+\\.js$`).test(f));
    if (matches.length !== 1) {
      throw new Error(`expected one built chunk named ${name} (awaited before first render); found ${matches.length}`);
    }
    roots.push(matches[0]!);
  }
  const seen = new Set<string>();
  const visit = (file: string) => {
    if (seen.has(file)) return;
    const code = chunks[file];
    if (code === undefined) throw new Error(`${file} is imported at start-up but was not built`);
    seen.add(file);
    for (const dep of staticImports(code)) visit(dep);
  };
  roots.forEach(visit);
  return [...seen];
}
