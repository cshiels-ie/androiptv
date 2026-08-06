import { defineConfig } from "vite";

// TV page build: single-file bundle (no code splitting, everything
// inlined including hls.js) so the Rust backend can embed it with
// include_str! and serve it with zero external asset requests —
// TV browsers (Samsung Internet on older Tizen) are weak JS engines.
export default defineConfig({
  root: "tv",
  build: {
    outDir: "../dist-tv", // relative to root ("tv")
    emptyOutDir: true,
    target: "es2017", // conservative: older Tizen Chromium
    cssCodeSplit: false,
    assetsInlineLimit: 100000000, // inline everything
    rollupOptions: {
      // Vite resolves HTML entry paths against the project root (cwd),
      // so use the explicit path even though root is "tv".
      input: "tv/index.html",
      output: {
        inlineDynamicImports: true,
      },
    },
  },
});
