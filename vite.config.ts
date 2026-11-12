import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";
import process from "node:process";
import JavaScriptObfuscator from "javascript-obfuscator";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(({ mode }) => ({
  define: {
    __RBL_MOCK_BACKEND__: JSON.stringify(mode !== "production" || process.env.RBL_MOCK === "1"),
  },
  plugins: [react(), {
    name: "obfuscate-production",
    apply: "build",
    enforce: "post",
    // Transform before Rollup finalizes hashes and cross-chunk imports.
    renderChunk(code, chunk) {
      if (chunk.name === "vendor") return null;
      const obfuscated = JavaScriptObfuscator.obfuscate(code, {
        target: "browser-no-eval",
        seed: 1,
        compact: true,
        identifierNamesGenerator: "hexadecimal",
        renameGlobals: false,
        renameProperties: false,
        // Vite discovers lazy chunks' CSS dependencies after renderChunk.
        // Keep import paths literal so that discovery survives obfuscation.
        ignoreImports: true,
        // Preserve CSP and keep the deck/canvas hot paths inexpensive.
        controlFlowFlattening: false,
        deadCodeInjection: false,
        debugProtection: false,
        selfDefending: false,
        disableConsoleOutput: false,
        stringArray: true,
        // Keep shuffled string tables, but avoid per-chunk startup rotation
        // loops and their duplicated code as stage feedback adds lazy chunks.
        stringArrayRotate: false,
        stringArrayIndexShift: false,
        stringArrayThreshold: 0.5,
        stringArrayEncoding: [],
      });
      return { code: obfuscated.getObfuscatedCode(), map: null };
    },
  }],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
    // To evaluate Preact as a smaller compatibility layer, add:
    //   react: "preact/compat", "react-dom": "preact/compat"
  },
  clearScreen: false,
  build: {
    // WKWebView (macOS 13+) and WebView2 (Edge 110+) are the only targets we ship to.
    target: ["safari16", "edge110"],
    rollupOptions: {
      output: {
        manualChunks(id) {
          // Third-party code gains no protection from obfuscation.
          if (id.includes("/node_modules/")) return "vendor";
          // keep canvas + device views off the cold-start path
          if (id.includes("/src/canvas/")) return "canvas";
        },
      },
    },
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
}));
