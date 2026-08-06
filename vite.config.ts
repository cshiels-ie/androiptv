import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Main app build (desktop/phone UI). The TV page has its own entry
// (vite.tv.config.ts) so it compiles to a single inline HTML file
// that gets embedded into the Rust binary via include_str!.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    outDir: "dist",
    target: "es2021",
  },
});
