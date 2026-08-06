// gen-icons.mjs — dependency-free PNG/ICO icon generator.
// Writes src-tauri/icons/{32x32.png, 128x128.png, 128x128@2x.png, icon.png, icon.ico}.
// Uses only node:zlib + node:fs: PNG encoding (8-bit RGBA, color type 6) is
// implemented by hand, the .ico embeds a 256px PNG directly.

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const OUT_DIR = join(dirname(fileURLToPath(import.meta.url)), "..", "src-tauri", "icons");

// --- CRC32 (table + chunk framing) -------------------------------------------
const CRC_TABLE = new Int32Array(256);
for (let n = 0; n < 256; n++) {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  CRC_TABLE[n] = c;
}

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const typeBuf = Buffer.from(type, "ascii");
  const crcBuf = Buffer.alloc(4);
  crcBuf.writeUInt32BE(crc32(Buffer.concat([typeBuf, data])));
  return Buffer.concat([len, typeBuf, data, crcBuf]);
}

// --- PNG encoder: 8-bit RGBA, color type 6, single IDAT ----------------------
function encodePng(w, h, rgba) {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type: RGBA
  // compression 0, filter 0, interlace 0 (already zeroed)

  const stride = w * 4;
  const raw = Buffer.alloc((stride + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (stride + 1)] = 0; // filter: none
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), // PNG signature
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// --- Artwork ----------------------------------------------------------------
const BG = [0x0b, 0x0f, 0x14]; // dark navy
const TOP = [0x60, 0xa5, 0xfa]; // accent blue, light end
const BOT = [0x3b, 0x82, 0xf6]; // accent blue, dark end

function insideTriangle(px, py, a, b, c) {
  // Same-sign cross-product test (pixel center counts as inside on the edge).
  const s1 = (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0]);
  const s2 = (c[0] - b[0]) * (py - b[1]) - (c[1] - b[1]) * (px - b[0]);
  const s3 = (a[0] - c[0]) * (py - c[1]) - (a[1] - c[1]) * (px - c[0]);
  return (s1 >= 0 && s2 >= 0 && s3 >= 0) || (s1 <= 0 && s2 <= 0 && s3 <= 0);
}

function draw(size) {
  const buf = Buffer.alloc(size * size * 4);
  // Play triangle: (0.38w, 0.28h) — (0.38w, 0.72h) — (0.75w, 0.5h)
  const a = [0.38 * size, 0.28 * size];
  const b = [0.38 * size, 0.72 * size];
  const c = [0.75 * size, 0.5 * size];

  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const i = (y * size + x) * 4;
      const px = x + 0.5;
      const py = y + 0.5;
      if (insideTriangle(px, py, a, b, c)) {
        // Vertical gradient over the triangle's own height.
        const t = Math.min(1, Math.max(0, (py - a[1]) / (b[1] - a[1])));
        buf[i] = Math.round(TOP[0] + (BOT[0] - TOP[0]) * t);
        buf[i + 1] = Math.round(TOP[1] + (BOT[1] - TOP[1]) * t);
        buf[i + 2] = Math.round(TOP[2] + (BOT[2] - TOP[2]) * t);
        buf[i + 3] = 255;
      } else {
        buf[i] = BG[0];
        buf[i + 1] = BG[1];
        buf[i + 2] = BG[2];
        buf[i + 3] = 255;
      }
    }
  }
  return buf;
}

// --- ICO: 1-entry container with the 256px PNG embedded ---------------------
function toIco(png256) {
  const header = Buffer.alloc(6 + 16); // ICONDIR + one ICONDIRENTRY
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(1, 4); // count: 1
  header[6] = 0; // width 256 -> stored as 0
  header[7] = 0; // height 256 -> stored as 0
  header[8] = 0; // palette colors
  header[9] = 0; // reserved
  header.writeUInt16LE(1, 10); // color planes
  header.writeUInt16LE(32, 12); // bits per pixel
  header.writeUInt32LE(png256.length, 14); // bytes in resource
  header.writeUInt32LE(22, 18); // image offset (right after header)
  return Buffer.concat([header, png256]);
}

// --- Emit --------------------------------------------------------------------
mkdirSync(OUT_DIR, { recursive: true });

const pngFiles = [
  ["32x32.png", 32],
  ["128x128.png", 128],
  ["128x128@2x.png", 256],
  ["icon.png", 512],
];

const written = [];
for (const [name, size] of pngFiles) {
  const path = join(OUT_DIR, name);
  writeFileSync(path, encodePng(size, size, draw(size)));
  written.push(path);
}

const icoPath = join(OUT_DIR, "icon.ico");
writeFileSync(icoPath, toIco(encodePng(256, 256, draw(256)))); // ico embeds a 256px PNG
written.push(icoPath);

for (const path of written) console.log(path);
