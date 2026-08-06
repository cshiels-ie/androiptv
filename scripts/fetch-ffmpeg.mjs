// fetch-ffmpeg.mjs — downloads a static ffmpeg build for the CURRENT platform
// into src-tauri/binaries/ using Tauri's externalBin naming convention:
//   ffmpeg-<rust-target-triple>   (plus .exe on Windows)
// Tauri resolves the `"externalBin": ["binaries/ffmpeg"]` entry in
// tauri.conf.json to src-tauri/binaries/ffmpeg-<triple>[.exe] at build time.
//
// - Skips silently if the binary already exists.
// - Android: no download — the README explains building once with
//   ffmpeg-android-maker (https://github.com/Javernaut/ffmpeg-android-maker).
// - On failure prints a clear message; exits 1 (the desktop bundle genuinely
//   needs the binary because tauri.conf.json declares it as externalBin).

import {
  chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync,
  readdirSync, rmSync, writeFileSync,
} from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DEST_DIR = join(ROOT, "src-tauri", "binaries");

// --- Platform → target triple -----------------------------------------------
const platform = process.platform;
const arch = process.arch;

function detectTriple() {
  if (platform === "win32") return "x86_64-pc-windows-msvc";
  if (platform === "darwin") return arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
  if (platform === "linux") return arch === "arm64" ? "aarch64-unknown-linux-gnu" : "x86_64-unknown-linux-gnu";
  return null;
}

// --- Download sources per triple --------------------------------------------
const SOURCES = {
  "x86_64-unknown-linux-gnu": {
    url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz",
    exe: "ffmpeg",
  },
  "aarch64-unknown-linux-gnu": {
    url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz",
    exe: "ffmpeg",
  },
  "x86_64-pc-windows-msvc": {
    url: "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip",
    exe: "ffmpeg.exe",
  },
  "aarch64-apple-darwin": {
    url: "https://evermeet.cx/ffmpeg/getrelease/zip",
    exe: "ffmpeg",
  },
  "x86_64-apple-darwin": {
    url: "https://evermeet.cx/ffmpeg/getrelease/zip",
    exe: "ffmpeg",
  },
};

// --- Helpers -----------------------------------------------------------------
function extractTarXz(archive, tmp) {
  // node:zlib cannot inflate .xz; `tar -xJf` is present on Linux/macOS.
  execFileSync("tar", ["-xJf", archive, "-C", tmp]);
}

function extractZip(archive, tmp) {
  try {
    execFileSync("unzip", ["-o", archive, "-d", tmp]);
  } catch {
    // Fallback: Windows 10+ ships bsdtar, which reads zip archives too.
    execFileSync("tar", ["-xf", archive, "-C", tmp]);
  }
}

// Find the binary anywhere under dir (handles the version-stripped layouts:
// ffmpeg-master-latest-linux64-gpl/bin/ffmpeg, plain ./ffmpeg, ...).
function findBinary(dir, name) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) {
      const found = findBinary(p, name);
      if (found) return found;
    } else if (entry.name === name) {
      return p;
    }
  }
  return null;
}

// --- Main --------------------------------------------------------------------
const android = platform === "android";
const triple = detectTriple();

if (android) {
  console.log(
    "Android: no ffmpeg download here — static executables must be built once with\n" +
      "ffmpeg-android-maker (https://github.com/Javernaut/ffmpeg-android-maker) and placed\n" +
      "at src-tauri/binaries/ffmpeg-<abi-triple> (e.g. ffmpeg-aarch64-linux-android).\n" +
      "See README.md → Android. Nothing to fetch."
  );
  process.exit(0);
}

if (!triple || !SOURCES[triple]) {
  console.error(`Unsupported platform: ${platform}/${arch} — run this script on Windows, macOS or Linux (or handle Android separately).`);
  process.exit(1);
}

const isWindows = platform === "win32";
const target = join(DEST_DIR, `ffmpeg-${triple}${isWindows ? ".exe" : ""}`);

if (existsSync(target)) {
  console.log(`already present: ${target} — skipping.`);
} else {
  const src = SOURCES[triple];
  const tmp = mkdtempSync(join(tmpdir(), "ffmpeg-"));
  try {
    console.log(`downloading ${src.url} …`);
    const res = await fetch(src.url);
    if (!res.ok) throw new Error(`HTTP ${res.status} ${res.statusText}`);
    const buf = Buffer.from(await res.arrayBuffer());
    console.log(`downloaded ${(buf.length / 1024 / 1024).toFixed(1)} MiB`);

    const archive = join(tmp, "ffmpeg-download");
    writeFileSync(archive, buf);
    src.url.endsWith(".tar.xz") ? extractTarXz(archive, tmp) : extractZip(archive, tmp);

    const found = findBinary(tmp, src.exe);
    if (!found) throw new Error(`could not locate ${src.exe} inside the archive`);

    mkdirSync(DEST_DIR, { recursive: true });
    copyFileSync(found, target);
    if (!isWindows) chmodSync(target, 0o755); // make executable on posix
    console.log(`installed ${target}`);
  } catch (err) {
    console.error(`failed to fetch ffmpeg for ${triple}: ${err.message}`);
    console.error(`desktop bundles require the binary (externalBin in tauri.conf.json).`);
    console.error(`Download it manually and place it at: ${target}`);
    process.exit(1);
  } finally {
    rmSync(tmp, { recursive: true, force: true }); // never leave a half-download
  }
}

// --- Instructions for other platforms ---------------------------------------
console.log("\nOther platforms (run this script there, or fetch manually):");
for (const [t, s] of Object.entries(SOURCES)) {
  if (t !== triple) console.log(`  ${t}  ←  ${s.url}`);
}
console.log("\nAndroid: build static ffmpeg with ffmpeg-android-maker — see README.md.");
