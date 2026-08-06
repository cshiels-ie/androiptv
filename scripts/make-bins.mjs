// Creates node_modules/.bin shims as REGULAR FILES (not symlinks).
// This filesystem rejects symlinks, so npm's bin-links are disabled
// (.npmrc) and this postinstall hook generates equivalent shims.
// Each shim spawns node with the package's real bin entry.
import { readdirSync, existsSync, readFileSync, writeFileSync, chmodSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const mods = join(process.cwd(), "node_modules");
const binDir = join(mods, ".bin");
mkdirSync(binDir, { recursive: true });

// Collect every package directory (descend into @scope dirs,
// whose children are the actual packages) with its package.json.
const pkgs = [];
for (const name of readdirSync(mods)) {
  if (name.startsWith(".")) continue;
  const scopeDir = join(mods, name);
  if (name.startsWith("@")) {
    if (!existsSync(join(scopeDir, "package.json"))) {
      for (const sub of readdirSync(scopeDir)) {
        const pkgJson = join(scopeDir, sub, "package.json");
        if (existsSync(pkgJson)) pkgs.push(join(name, sub), pkgJson);
      }
      continue;
    }
  }
  const pkgJson = join(scopeDir, "package.json");
  if (existsSync(pkgJson)) pkgs.push(name, pkgJson);
}

let count = 0;
for (let i = 0; i < pkgs.length; i += 2) {
  const name = pkgs[i];
  const pkgJson = pkgs[i + 1];
  let pkg;
  try {
    pkg = JSON.parse(readFileSync(pkgJson, "utf8"));
  } catch {
    continue;
  }
  const bin = pkg.bin;
  if (!bin) continue;
  // A string `bin` names the executable after the package; for scoped
  // packages npm uses the basename (e.g. `@babel/parser` → `parser`).
  // Slashes in a bin name would point outside .bin, so strip the scope.
  const entries = typeof bin === "string" ? { [name.slice(name.lastIndexOf("/") + 1)]: bin } : bin;
  for (const [binName, target] of Object.entries(entries)) {
    if (binName.includes("/")) continue;
    if (!existsSync(join(mods, name, target))) continue;
    const relTarget = join("..", name, target).split("\\").join("/");
    const shim = join(binDir, binName);
    // Skip bins that already exist (e.g. npm created real bin-links on a
    // symlink-capable filesystem): overwriting would clobber the link or,
    // worse, write through the link onto the package's real bin file.
    if (existsSync(shim) || existsSync(`${shim}.cmd`)) continue;
    // If the target is a native binary (e.g. esbuild after its
    // install.js replaced the JS wrapper), exec it directly; JS/script
    // targets run under node.
    const body = `#!/usr/bin/env node
const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const target = path.join(__dirname, ${JSON.stringify(relTarget)});
const fd = fs.openSync(target, "r");
const buf = Buffer.alloc(2);
fs.readSync(fd, buf, 0, 2, 0);
fs.closeSync(fd);
const isText = buf[0] === 0x23; // '#'
const r = isText
  ? spawnSync(process.execPath, [target, ...process.argv.slice(2)], { stdio: "inherit" })
  : spawnSync(target, process.argv.slice(2), { stdio: "inherit" });
process.exit(r.status ?? 1);
`;
    writeFileSync(shim, body);
    chmodSync(shim, 0o755);
    // Windows twin (cmd.exe can't execute the POSIX shim)
    const cmdShim = join(binDir, `${binName}.cmd`);
    const cmdBody = `@echo off\r\nnode "%~dp0${relTarget}" %*\r\n`;
    writeFileSync(cmdShim, cmdBody);
    count++;
  }
}
console.log(`make-bins: wrote ${count} shims into node_modules/.bin`);
