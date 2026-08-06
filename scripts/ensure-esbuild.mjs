// Ensures esbuild's platform binary is in place.
//
// Two known failure modes on Android-storage filesystems (no symlinks,
// no tar-hardlink materialization):
//   1. npm's install-script ordering races esbuild's optional-dep
//      extraction -> use `npm install --ignore-scripts` on such hosts.
//   2. @esbuild/<platform> tarballs ship the native binary as a tar
//      HARDLINK entry; extraction leaves a temp-named file (".l2s.*")
//      instead of "bin/esbuild". We copy the content into place.
//
// Idempotent: no-ops when everything is already fine.
import { existsSync, readdirSync, statSync, copyFileSync, chmodSync } from "node:fs";
import { join } from "node:path";
import { execFileSync } from "node:child_process";

const mods = join(process.cwd(), "node_modules");
const esbuildDir = join(mods, "esbuild");
const binJs = join(esbuildDir, "bin", "esbuild");
const installJs = join(esbuildDir, "install.js");

if (!existsSync(binJs)) {
  console.log("ensure-esbuild: esbuild not installed, skipping");
  process.exit(0);
}

// Repair platform packages whose native binary failed to materialize.
const esbuildScope = join(mods, "@esbuild");
if (existsSync(esbuildScope)) {
  for (const pkg of readdirSync(esbuildScope)) {
    const binDir = join(esbuildScope, pkg, "bin");
    if (!existsSync(binDir)) continue;
    const expected = join(binDir, "esbuild");
    if (existsSync(expected)) continue;
    // Pick the largest temp-named file as the binary content.
    const candidates = readdirSync(binDir)
      .filter((f) => f !== "." && f !== "..")
      .map((f) => join(binDir, f))
      .filter((f) => statSync(f).isFile());
    if (candidates.length === 0) continue;
    candidates.sort((a, b) => statSync(b).size - statSync(a).size);
    copyFileSync(candidates[0], expected);
    chmodSync(expected, 0o755);
    console.log(`ensure-esbuild: repaired ${pkg}/bin/esbuild (from ${join("bin", candidates[0].split("bin").pop() ?? "")})`);
  }
}

// Validate like esbuild's own install.js would.
try {
  execFileSync("node", [binJs, "--version"], { stdio: "pipe" });
  console.log("ensure-esbuild: ok");
} catch {
  // install.js re-validates and sets up any remaining pieces
  try {
    execFileSync("node", [installJs], { stdio: "inherit", cwd: esbuildDir });
    console.log("ensure-esbuild: ok (after install.js)");
  } catch {
    console.error("ensure-esbuild: failed to prepare esbuild binary");
    process.exit(1);
  }
}
