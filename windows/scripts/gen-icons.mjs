// Renders the app icons Tauri bundles from Glim's brand artwork, through the
// Tauri CLI's own `icon` command, and keeps only the files tauri.conf.json
// lists.
//
// docs/brand/app-icon.png is the main icon. docs/brand/app-icon-small.png is
// drawn for small sizes, so the 16, 24 and 32 px layers of icon.ico and
// 32x32.png come from it instead of a downscale of the main icon.
//
//   npm run icons

import { execSync } from "node:child_process";
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const brand = resolve(root, "..", "docs", "brand");
const out = join(root, "src-tauri", "icons");
const FROM_MAIN = ["128x128.png", "128x128@2x.png", "icon.png"];
const SMALL_ICO_SIZES = new Set([16, 24, 32]);

// An .ico is a 6-byte header, a 16-byte entry per image, then the images.
// Byte 0 of an entry is the width (0 means 256).
function readIco(path) {
  const b = readFileSync(path);
  const images = [];
  for (let i = 0; i < b.readUInt16LE(4); i++) {
    const e = 6 + 16 * i;
    const size = b[e] || 256;
    images.push({ size, entry: b.subarray(e, e + 16), data: b.subarray(b.readUInt32LE(e + 12), b.readUInt32LE(e + 12) + b.readUInt32LE(e + 8)) });
  }
  return images;
}

function writeIco(path, images) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  let offset = 6 + 16 * images.length;
  const entries = images.map(({ entry, data }) => {
    const e = Buffer.from(entry);
    e.writeUInt32LE(data.length, 8);
    e.writeUInt32LE(offset, 12);
    offset += data.length;
    return e;
  });
  writeFileSync(path, Buffer.concat([header, ...entries, ...images.map((i) => i.data)]));
}

const tmp = mkdtempSync(join(tmpdir(), "glim-icons-"));
try {
  const main = join(tmp, "main");
  const small = join(tmp, "small");
  execSync(`npx tauri icon "${join(brand, "app-icon.png")}" -o "${main}"`, { cwd: root, stdio: "inherit" });
  execSync(`npx tauri icon "${join(brand, "app-icon-small.png")}" -o "${small}"`, { cwd: root, stdio: "inherit" });
  for (const name of FROM_MAIN) copyFileSync(join(main, name), join(out, name));
  copyFileSync(join(small, "32x32.png"), join(out, "32x32.png"));
  const smallLayers = readIco(join(small, "icon.ico"));
  const layers = readIco(join(main, "icon.ico")).map((img) =>
    SMALL_ICO_SIZES.has(img.size) ? smallLayers.find((s) => s.size === img.size) : img,
  );
  writeIco(join(out, "icon.ico"), layers);
  console.log(`Wrote ${[...FROM_MAIN, "32x32.png", "icon.ico"].join(", ")} to ${out}`);
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
