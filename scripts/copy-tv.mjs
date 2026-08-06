// copy-tv.mjs — inlines the built TV bundle into the Rust backend resources.
// The TV page is built separately (vite.tv.config.ts → dist-tv/index.html) and
// embedded into the binary via include_str!("../../resources/tv/index.html"),
// so it can be served same-origin by the embedded axum server.
// The JS/CSS chunks are inlined into the HTML here so the embedded page is a
// single self-contained file with zero external asset requests — the axum
// server has no /assets route, and weak TV browsers (older Tizen) prefer it.

import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const src = join(ROOT, "dist-tv", "index.html");
const destDir = join(ROOT, "src-tauri", "resources", "tv");
const dest = join(destDir, "index.html");

if (!existsSync(src)) {
  console.error(
    "dist-tv/index.html not found — run `npm run tv:build` first so the " +
      "tv/ page is bundled into dist-tv, then re-run this script."
  );
  process.exit(1);
}

mkdirSync(destDir, { recursive: true });

const html = readFileSync(src, "utf8");

// Inline emitted <script src="/assets/..."> and <link href="/assets/...">
// chunks. Vite hashes these filenames per build, so resolve them by reading
// the src/href attributes out of the built HTML itself.
const inlined = html.replace(
  /<script type="module" crossorigin src="(\/[^"]+)"><\/script>/g,
  (_m, asset) => {
    const body = readFileSync(join(ROOT, "dist-tv", asset.replace(/^\//, "")), "utf8");
    return `<script type="module">\n${body}\n</script>`;
  }
).replace(
  /<link rel="stylesheet" crossorigin href="(\/[^"]+)">/g,
  (_m, asset) => {
    const body = readFileSync(join(ROOT, "dist-tv", asset.replace(/^\//, "")), "utf8");
    return `<style>\n${body}\n</style>`;
  }
);

if (inlined === html) {
  console.warn(
    "copy-tv: no <script src> or <link stylesheet> found in dist-tv/index.html — " +
      "the embedded page may reference missing /assets files."
  );
}

writeFileSync(dest, inlined);
console.log(`inlined ${src} → ${dest} (${Buffer.byteLength(inlined)} bytes)`);
