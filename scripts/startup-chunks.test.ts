import { describe, expect, it } from "vitest";
import { startupChunks } from "./startup-chunks";

const html = `<!doctype html><html><head>
<script src="/os.js"></script>
<script type="module" crossorigin src="/assets/index-AAAA1111.js"></script>
<link rel="modulepreload" crossorigin href="/assets/vendor-BBBB2222.js">
<link rel="stylesheet" crossorigin href="/assets/index-CCCC3333.css">
</head><body><div id="root"></div></body></html>`;

describe("startupChunks", () => {
  it("counts the entry and everything it imports statically, transitively", () => {
    const chunks = {
      "index-AAAA1111.js": `import{a as r}from"./vendor-BBBB2222.js";import"./theme-DDDD4444.js";r();`,
      "vendor-BBBB2222.js": `export const a=()=>1;`,
      "theme-DDDD4444.js": `import{x}from"./tokens-EEEE5555.js";x;`,
      "tokens-EEEE5555.js": `export const x=1;`,
      "Lazy-FFFF6666.js": `export const z=1;`,
    };
    expect(startupChunks(html, chunks, []).sort()).toEqual([
      "index-AAAA1111.js",
      "theme-DDDD4444.js",
      "tokens-EEEE5555.js",
      "vendor-BBBB2222.js",
    ]);
  });

  it("counts the chunk awaited before first render and its static imports, not other dynamic imports", () => {
    const chunks = {
      "index-AAAA1111.js": `import{a}from"./vendor-BBBB2222.js";const m=()=>import("./App-GGGG7777.js");const p=()=>import("./Prefs-HHHH8888.js");`,
      "vendor-BBBB2222.js": `export const a=1;`,
      "App-GGGG7777.js": `import{b as c}from"./icons-IIII9999.js";import{a}from"./index-AAAA1111.js";const i=()=>import("./IntakeView-JJJJ0000.js");`,
      "icons-IIII9999.js": `export const b=1;`,
      "IntakeView-JJJJ0000.js": `import"./big-KKKK1111.js";`,
      "big-KKKK1111.js": `export{};`,
      "Prefs-HHHH8888.js": `export{};`,
    };
    expect(startupChunks(html, chunks, ["App"]).sort()).toEqual([
      "App-GGGG7777.js",
      "icons-IIII9999.js",
      "index-AAAA1111.js",
      "vendor-BBBB2222.js",
    ]);
  });

  it("refuses to measure when the awaited chunk is missing, so a rename cannot shrink start-up to the entry", () => {
    const chunks = { "index-AAAA1111.js": `export{};`, "vendor-BBBB2222.js": `export{};`, "Main-GGGG7777.js": `export{};` };
    expect(() => startupChunks(html, chunks, ["App"])).toThrow(/one built chunk named App/);
  });

  it("refuses an index.html without a module entry", () => {
    expect(() => startupChunks("<html></html>", { "index-AAAA1111.js": "" }, [])).toThrow(/no module entry/);
  });
});

